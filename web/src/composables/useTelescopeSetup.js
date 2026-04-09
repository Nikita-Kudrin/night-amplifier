import {ref, computed, watch, inject} from 'vue'
import {updateSettings, updatePushToConfig} from './api.js'

/**
 * Composable for telescope setup and FOV calculation.
 *
 * Manages focal length, camera sensor selection (pixel size + resolution),
 * and barlow/reducer coefficient. Computes field of view and sends it
 * to the plate solver whenever the parameters change.
 *
 * @param {Object} options
 * @param {Function} options.withErrorHandling - Error handling wrapper
 * @returns Reactive telescope state, computed FOV, and helper methods
 */
export function useTelescopeSetup({withErrorHandling} = {}) {
  const settings = inject('settings')

  // ── Local reactive state ──────────────────────────────────────────
  const focalLength = ref(null)
  const pixelSizeX = ref(null)
  const pixelSizeY = ref(null)
  const sensorWidthPx = ref(null)
  const sensorHeightPx = ref(null)
  const barlowCoeff = ref(1.0)
  const manualPixelSize = ref(false)

  // ── Sync from persisted settings on load ──────────────────────────
  let initialSyncDone = false

  watch(settings, (s) => {
    if (!s?.telescope || initialSyncDone) return
    initialSyncDone = true
    const t = s.telescope
    if (t.focal_length_mm != null) focalLength.value = t.focal_length_mm
    if (t.pixel_size_x_um != null) pixelSizeX.value = t.pixel_size_x_um
    if (t.pixel_size_y_um != null) pixelSizeY.value = t.pixel_size_y_um
    if (t.sensor_width_px != null) sensorWidthPx.value = t.sensor_width_px
    if (t.sensor_height_px != null) sensorHeightPx.value = t.sensor_height_px
    if (t.barlow_coeff != null) barlowCoeff.value = t.barlow_coeff
  }, {immediate: true})

  // ── FOV calculation ───────────────────────────────────────────────
  const calculatedFov = computed(() => {
    const fl = focalLength.value
    const px = pixelSizeX.value
    const py = pixelSizeY.value
    const w = sensorWidthPx.value
    const h = sensorHeightPx.value
    const bc = barlowCoeff.value || 1.0

    if (!fl || fl <= 0 || !px || px <= 0 || !py || py <= 0 || !w || w <= 0 || !h || h <= 0) {
      return null
    }

    const effectiveFl = fl * bc
    const fovXDeg = (w * px / 1000.0) / effectiveFl * (180.0 / Math.PI)
    const fovYDeg = (h * py / 1000.0) / effectiveFl * (180.0 / Math.PI)
    return {x: fovXDeg, y: fovYDeg}
  })

  // ── Persist and send FOV to ASTAP when params change ──────────────
  let saveTimer = null

  function scheduleSettingsSave() {
    clearTimeout(saveTimer)
    saveTimer = setTimeout(async () => {
      const telescope = {
        focal_length_mm: focalLength.value || null,
        pixel_size_x_um: pixelSizeX.value || null,
        pixel_size_y_um: pixelSizeY.value || null,
        sensor_width_px: sensorWidthPx.value || null,
        sensor_height_px: sensorHeightPx.value || null,
        barlow_coeff: barlowCoeff.value || null,
      }

      const doSave = async () => {
        await updateSettings({telescope})

        // Send calculated FOV to ASTAP
        const fov = calculatedFov.value
        if (fov) {
          const fovHint = Math.max(fov.x, fov.y)
          try {
            await updatePushToConfig({fov_degrees: fovHint})
          } catch {
            // Push-to config may fail if plugin not available; ignore
          }
        }
      }

      if (withErrorHandling) {
        await withErrorHandling(doSave)
      } else {
        await doSave()
      }
    }, 500)
  }

  // Watch all telescope params and debounce-save
  watch([focalLength, pixelSizeX, pixelSizeY, sensorWidthPx, sensorHeightPx, barlowCoeff], () => {
    if (initialSyncDone) {
      scheduleSettingsSave()
    }
  })

  // ── Auto-fill from connected camera ───────────────────────────────
  function autoFillFromCamera(cameraInfo) {
    if (!cameraInfo) return
    if (cameraInfo.pixel_size_x_um) pixelSizeX.value = cameraInfo.pixel_size_x_um
    if (cameraInfo.pixel_size_y_um) pixelSizeY.value = cameraInfo.pixel_size_y_um
    if (cameraInfo.max_width) sensorWidthPx.value = cameraInfo.max_width
    if (cameraInfo.max_height) sensorHeightPx.value = cameraInfo.max_height
    manualPixelSize.value = false
  }

  // ── Select from camera database ───────────────────────────────────
  function selectCamera(entry) {
    pixelSizeX.value = entry.pixel_size_x
    pixelSizeY.value = entry.pixel_size_y
    sensorWidthPx.value = entry.width
    sensorHeightPx.value = entry.height
    manualPixelSize.value = false
  }

  return {
    focalLength,
    pixelSizeX,
    pixelSizeY,
    sensorWidthPx,
    sensorHeightPx,
    barlowCoeff,
    manualPixelSize,
    calculatedFov,
    autoFillFromCamera,
    selectCamera,
  }
}
