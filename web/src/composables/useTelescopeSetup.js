import {ref, computed, watch, inject} from 'vue'
import {updateSettings, updatePushToConfig} from './api.js'
import {CAMERA_DATABASE} from '../constants/cameras.js'

/**
 * Composable for telescope setup and FOV calculation.
 *
 * Manages focal length, camera sensor selection (pixel size + resolution),
 * and barlow/reducer coefficient. Computes field of view and sends it
 * to the plate solver whenever the parameters change.
 *
 * When a camera is connected, automatically resolves telescope settings:
 * 1. Restore stored per-camera profile if this camera was seen before
 * 2. For new cameras: inherit focal_length/barlow from previous camera,
 *    fill pixel size from driver, fallback to CAMERA_DATABASE
 *
 * @param {Object} options
 * @param {Function} options.withErrorHandling - Error handling wrapper
 * @param {import('vue').Ref} options.connectedCameraInfo - Reactive camera info from selected camera
 * @returns Reactive telescope state, computed FOV, and helper methods
 */
export function useTelescopeSetup({withErrorHandling, connectedCameraInfo} = {}) {
    const settings = inject('settings')

    // ── Local reactive state ──────────────────────────────────────────
    const focalLength = ref(null)
    const pixelSizeX = ref(null)
    const pixelSizeY = ref(null)
    const sensorWidthPx = ref(null)
    const sensorHeightPx = ref(null)
    const barlowCoeff = ref(1.0)
    const manualPixelSize = ref(false)

    // ── Per-camera profile state ──────────────────────────────────────
    const cameraProfiles = ref({})   // camera_name -> TelescopeSettings
    const lastCameraName = ref(null)

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
        if (s.camera_telescope_profiles) {
            cameraProfiles.value = {...s.camera_telescope_profiles}
        }
        if (s.last_camera_name) {
            lastCameraName.value = s.last_camera_name
        }
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

    function buildTelescopePayload() {
        return {
            focal_length_mm: focalLength.value || null,
            pixel_size_x_um: pixelSizeX.value || null,
            pixel_size_y_um: pixelSizeY.value || null,
            sensor_width_px: sensorWidthPx.value || null,
            sensor_height_px: sensorHeightPx.value || null,
            barlow_coeff: barlowCoeff.value || null,
        }
    }

    function scheduleSettingsSave() {
        clearTimeout(saveTimer)
        saveTimer = setTimeout(async () => {
            const telescope = buildTelescopePayload()

            // Keep current camera's profile in sync
            if (lastCameraName.value) {
                cameraProfiles.value[lastCameraName.value] = {...telescope}
            }

            const doSave = async () => {
                await updateSettings({
                    telescope,
                    camera_telescope_profiles: cameraProfiles.value,
                    last_camera_name: lastCameraName.value,
                })

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

    // ── Per-camera profile helpers ────────────────────────────────────

    function saveCurrentAsProfile(cameraName) {
        cameraProfiles.value[cameraName] = buildTelescopePayload()
    }

    function restoreProfile(profile) {
        if (profile.focal_length_mm != null) focalLength.value = profile.focal_length_mm
        if (profile.pixel_size_x_um != null) pixelSizeX.value = profile.pixel_size_x_um
        if (profile.pixel_size_y_um != null) pixelSizeY.value = profile.pixel_size_y_um
        if (profile.sensor_width_px != null) sensorWidthPx.value = profile.sensor_width_px
        if (profile.sensor_height_px != null) sensorHeightPx.value = profile.sensor_height_px
        if (profile.barlow_coeff != null) barlowCoeff.value = profile.barlow_coeff
        manualPixelSize.value = false
    }

    /**
     * Match a camera name against the well-known CAMERA_DATABASE.
     * Tries exact model match first, then substring containment.
     */
    function matchCameraInDatabase(cameraName) {
        if (!cameraName) return null
        const normalized = cameraName.toLowerCase().trim()

        // Exact model match
        const exact = CAMERA_DATABASE.find(
            c => c.model.toLowerCase() === normalized
        )
        if (exact) return exact

        // Substring match: camera name contains the DB model or vice versa
        const substring = CAMERA_DATABASE.find(
            c => normalized.includes(c.model.toLowerCase())
                || c.model.toLowerCase().includes(normalized)
        )
        if (substring) return substring

        return null
    }

    // ── Auto-apply on camera change ───────────────────────────────────

    if (connectedCameraInfo) {
        watch(connectedCameraInfo, (newInfo, oldInfo) => {
            if (!newInfo) return

            const cameraName = newInfo.name
            if (!cameraName) return

            // Don't re-process if the same camera is re-selected
            if (oldInfo?.name === cameraName) return

            // Save current settings as a profile for the outgoing camera
            if (oldInfo?.name) {
                saveCurrentAsProfile(oldInfo.name)
            }

            // Step 1: Check if we have a stored profile for this camera
            const existingProfile = cameraProfiles.value[cameraName]
            if (existingProfile) {
                restoreProfile(existingProfile)
                lastCameraName.value = cameraName
                scheduleSettingsSave()
                return
            }

            // Step 2: New camera -- inherit focal_length and barlow from last camera
            const lastProfile = lastCameraName.value
                ? cameraProfiles.value[lastCameraName.value]
                : null

            if (lastProfile) {
                if (lastProfile.focal_length_mm != null) focalLength.value = lastProfile.focal_length_mm
                if (lastProfile.barlow_coeff != null) barlowCoeff.value = lastProfile.barlow_coeff
            }

            // Step 3: Fill pixel size and sensor dims from driver
            let pixelSizeFilled = false
            if (newInfo.pixel_size_x_um && newInfo.pixel_size_x_um > 0) {
                pixelSizeX.value = newInfo.pixel_size_x_um
                pixelSizeY.value = newInfo.pixel_size_y_um || newInfo.pixel_size_x_um
                pixelSizeFilled = true
            }
            if (newInfo.max_width && newInfo.max_width > 0) {
                sensorWidthPx.value = newInfo.max_width
                sensorHeightPx.value = newInfo.max_height
            }

            // Step 4: If pixel size is 0 from driver, try CAMERA_DATABASE
            if (!pixelSizeFilled) {
                const dbMatch = matchCameraInDatabase(cameraName)
                if (dbMatch) {
                    pixelSizeX.value = dbMatch.pixel_size_x
                    pixelSizeY.value = dbMatch.pixel_size_y
                    // Also use DB resolution if driver didn't provide it
                    if (!newInfo.max_width || newInfo.max_width === 0) {
                        sensorWidthPx.value = dbMatch.width
                        sensorHeightPx.value = dbMatch.height
                    }
                }
            }

            manualPixelSize.value = false
            lastCameraName.value = cameraName
            scheduleSettingsSave()
        }, {immediate: false})
    }

    // ── Manual auto-fill from connected camera (button) ───────────────
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
