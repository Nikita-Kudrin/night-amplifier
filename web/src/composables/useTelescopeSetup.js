import {ref, computed, watch} from 'vue'
import {useApplySettings, useSettings} from './appContext.js'
import {useDebouncedCall} from './useDebouncedCall.js'
import {runDirectly} from './useError.js'
import {fieldOfViewDeg} from '../utils/optics.js'
import {CAMERA_DATABASE} from '../constants/cameras.js'
import {SETTINGS_DEBOUNCE_MS} from '../constants'

const TELESCOPE_FIELDS = [
    'focal_length_mm',
    'pixel_size_x_um',
    'pixel_size_y_um',
    'sensor_width_px',
    'sensor_height_px',
    'barlow_coeff',
]

/**
 * Settings as a string that ignores key order, the server's 0-vs-null spelling and an
 * unset Barlow, which the page shows (as a placeholder) and the solver reads as 1x.
 */
function payloadKey({telescope, camera_telescope_profiles: profiles, last_camera_name}) {
    const cameras = Object.keys(profiles ?? {}).sort()
    return JSON.stringify([
        opticsFields(telescope),
        cameras.map((name) => [name, opticsFields(profiles[name])]),
        last_camera_name ?? null,
    ])
}

function opticsFields(t) {
    return TELESCOPE_FIELDS.map((field) => t?.[field] || (field === 'barlow_coeff' ? 1 : null))
}

function sameOptics(a, b) {
    return JSON.stringify(opticsFields(a)) === JSON.stringify(opticsFields(b))
}

/**
 * The profiles that differ from what the server holds. The server upserts the ones a
 * request names; sending the whole map from the last fetch erased a profile seeded at
 * connect since, and the guide camera then solved on the imaging scope's optics.
 */
function changedProfiles(profiles, persisted) {
    return Object.fromEntries(
        Object.entries(profiles).filter(([name, profile]) => !sameOptics(profile, persisted[name]))
    )
}

/**
 * Telescope setup + FOV calculation: focal length, sensor (pixel size/resolution), and
 * barlow coefficient, persisted as the solver's FOV hint; auto-resolves from the camera on connect.
 * @param {Object} options
 * @param {Function} [options.withErrorHandling] - Error handling wrapper; errors propagate without one
 * @param {import('vue').Ref} options.connectedCameraInfo - Reactive camera info from selected camera
 * @returns Reactive telescope state, computed FOV, and helper methods
 */
export function useTelescopeSetup({withErrorHandling = runDirectly, connectedCameraInfo} = {}) {
    const settings = useSettings()
    const applySettings = useApplySettings()

    // ── Local reactive state ──────────────────────────────────────────
    const focalLength = ref(null)
    const pixelSizeX = ref(null)
    const pixelSizeY = ref(null)
    const sensorWidthPx = ref(null)
    const sensorHeightPx = ref(null)
    const barlowCoeff = ref(null)
    const manualPixelSize = ref(false)

    // ── Per-camera profile state ──────────────────────────────────────
    const cameraProfiles = ref({})   // camera_name -> TelescopeSettings
    const lastCameraName = ref(null)
    // The server's profiles as of the last sync or landed save: a save sends only the
    // entries that differ from them.
    let persistedProfiles = {}

    // ── Sync from the server's settings ───────────────────────────────
    // The Push-To tab stays mounted for the page's life, so this follows every settings
    // change: the server seeds a profile for each camera it connects, and other clients
    // edit them. Reading them once overwrote a guide scope's profile with the main
    // scope's focal length on the next camera click.
    const initialSyncDone = ref(false)
    // What the server holds, so re-deriving its values (a mount, a camera click, a sync)
    // never writes them back.
    let persistedKey = null
    // An edit typed here and not yet sent survives a sync; values derived from the
    // camera or the server do not. Set by a watcher that skips `assign`ed changes.
    let userEdited = false
    let assigning = 0
    // Outgoing cameras whose typed values were parked in `cameraProfiles` by a camera
    // switch inside the save delay.
    const editedProfiles = new Set()
    // Settings read before a save landed carry the old values, so a sync arriving while
    // one is in flight is held until it settles.
    let savesInFlight = 0
    let heldSync = null

    function assign(update) {
        assigning++
        try {
            update()
        } finally {
            assigning--
        }
    }

    // Whole, never field by field: the server seeds a camera on another sensor without a
    // focal length, and skipping the gap kept the previous scope's focal length on screen,
    // from where the next save wrote it into the new camera's profile.
    function applyFields(t) {
        focalLength.value = t.focal_length_mm ?? null
        pixelSizeX.value = t.pixel_size_x_um ?? null
        pixelSizeY.value = t.pixel_size_y_um ?? null
        sensorWidthPx.value = t.sensor_width_px ?? null
        sensorHeightPx.value = t.sensor_height_px ?? null
        // Unset stays unset (the input's placeholder shows 1x): posted back as 1x it read
        // as new optics, restarting the solve and orphaning the FOV measured for the rig.
        barlowCoeff.value = t.barlow_coeff ?? null
    }

    function syncFrom(s) {
        if (!s?.telescope) return
        if (savesInFlight) {
            heldSync = s
            return
        }
        const first = !initialSyncDone.value
        const serverProfiles = s.camera_telescope_profiles ?? {}
        const parked = Object.fromEntries(
            [...editedProfiles].map((name) => [name, cameraProfiles.value[name]])
        )
        cameraProfiles.value = {...serverProfiles, ...parked}
        persistedProfiles = serverProfiles
        persistedKey = payloadKey(s)

        if (!first && userEdited) return
        assign(() => {
            // The first sync loads the flat block; the camera watcher then resolves the
            // connected camera from it. Later ones follow that camera's own profile.
            const selected = connectedCameraInfo?.value?.name
            if (first || !selected) {
                applyFields(s.telescope)
                if (s.last_camera_name) lastCameraName.value = s.last_camera_name
            } else if (serverProfiles[selected]) {
                restoreProfile(serverProfiles[selected])
                lastCameraName.value = selected
            }
        })
        initialSyncDone.value = true
    }

    watch(settings, syncFrom, {immediate: true})

    // ── FOV calculation ───────────────────────────────────────────────
    const calculatedFov = computed(() => {
        const x = fieldOfViewDeg(sensorWidthPx.value, pixelSizeX.value, focalLength.value, barlowCoeff.value)
        const y = fieldOfViewDeg(sensorHeightPx.value, pixelSizeY.value, focalLength.value, barlowCoeff.value)
        return x === null || y === null ? null : {x, y}
    })

    // ── Persist when params change ────────────────────────────────────
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

    function buildSavePayload() {
        return {
            telescope: buildTelescopePayload(),
            camera_telescope_profiles: changedProfiles(cameraProfiles.value, persistedProfiles),
            last_camera_name: lastCameraName.value,
        }
    }

    async function saveSettings() {
        // Keep current camera's profile in sync
        if (lastCameraName.value) {
            cameraProfiles.value[lastCameraName.value] = buildTelescopePayload()
        }
        const payload = buildSavePayload()
        const key = payloadKey({...payload, camera_telescope_profiles: cameraProfiles.value})
        // Cleared before the request: anything typed while it is in flight is newer.
        const edited = userEdited
        const parked = [...editedProfiles]
        userEdited = false
        editedProfiles.clear()
        if (key === persistedKey) return
        savesInFlight++

        // Posting `telescope` is all the solver needs: the backend derives the image-
        // height FOV itself and prefers a previously measured FOV over any computed one.
        //
        // This used to also POST /push-to/config with `max(fovX, fovY)`, overwriting
        // that decision ms later — discarding a solved FOV for a computed one, using
        // the *width* field where ASTAP's `-fov` wants height. On a 2712x1538 sensor
        // that's a 1.76x error: a wrong FOV doesn't slow a hinted attempt, it fails it.
        let saved = false
        const doSave = async () => {
            await applySettings(payload)
            persistedKey = key
            persistedProfiles = {...persistedProfiles, ...payload.camera_telescope_profiles}
            saved = true
        }

        try {
            await withErrorHandling(doSave)
        } finally {
            savesInFlight--
            // A refused save keeps its edits safe from the next sync.
            if (!saved) {
                userEdited ||= edited
                parked.forEach((name) => editedProfiles.add(name))
            }
            releaseHeldSync(saved)
        }
    }

    function releaseHeldSync(saved) {
        if (savesInFlight || !heldSync) return
        const held = heldSync
        heldSync = null
        // After a refused save the held settings are still the server's, and the edits
        // are kept from them. After a landed one they may predate it, but the settings
        // store has adopted the save's answer: what it holds now is no older than the save.
        syncFrom(saved ? settings.value : held)
    }

    const {schedule: scheduleSettingsSave} = useDebouncedCall(saveSettings, SETTINGS_DEBOUNCE_MS)

    const telescopeParams = [focalLength, pixelSizeX, pixelSizeY, sensorWidthPx, sensorHeightPx, barlowCoeff]

    // Synchronous, so `assigning` still says who made the change.
    watch(telescopeParams, () => {
        if (!assigning) userEdited = true
    }, {flush: 'sync'})

    // Watch all telescope params and debounce-save
    watch(telescopeParams, () => {
        if (initialSyncDone.value) {
            scheduleSettingsSave()
        }
    })

    // ── Per-camera profile helpers ────────────────────────────────────

    function saveCurrentAsProfile(cameraName) {
        cameraProfiles.value[cameraName] = buildTelescopePayload()
        if (userEdited) editedProfiles.add(cameraName)
    }

    function restoreProfile(profile) {
        applyFields(profile)
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
        watch(
            [connectedCameraInfo, () => initialSyncDone.value],
            // Everything this derives is the camera's, not an edit typed here.
            ([newInfo, syncDone], oldValues) => assign(() => {
                if (!syncDone) return
                if (!newInfo) return

                const cameraName = newInfo.name
                if (!cameraName) return

                // `oldValues` is the array of previous watch sources, so the previous
                // camera info is its first element. Destructuring it in the parameter
                // list and then indexing again read `cameraInfo[0]`, which is always
                // undefined -- so the same-camera guard below never fired and the
                // outgoing camera's profile was never saved. The whole block then
                // re-ran on every `refreshCameras()`, re-applying driver defaults over
                // the values that had just been restored.
                const oldCameraName = oldValues?.[0]?.name

                // This watcher has two triggers, and they need opposite treatment.
                // Settings finishing their load is the *first* chance to resolve a
                // camera that connected before them, so it must run even though the
                // camera did not change. A camera list refresh for a camera we already
                // resolved must not.
                const justSynced = !oldValues?.[1]
                if (!justSynced && oldCameraName === cameraName) return

                // Save the outgoing camera's settings -- but only when there really is
                // an outgoing camera. On the initial sync the "old" camera is this
                // same one, and its current values have not been restored yet, so
                // saving here would overwrite the stored profile with defaults and
                // step 1 below would then restore what it had just destroyed.
                if (oldCameraName && oldCameraName !== cameraName) {
                    saveCurrentAsProfile(oldCameraName)
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
            }),
            {immediate: true}
        )
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
        // The camera whose profile the fields edit: the selected one, else the last one.
        profileName: computed(() => lastCameraName.value),
    }
}
