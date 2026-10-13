import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {ref, nextTick} from 'vue'
import {mount, flushPromises} from '@vue/test-utils'
import {useTelescopeSetup} from '../useTelescopeSetup.js'
import {SETTINGS_DEBOUNCE_MS} from '../../constants'
import * as api from '../api.js'
import {contextProvides} from '../appContext.js'

// Mock API
vi.mock('../api.js', async (importOriginal) =>
    (await import('../../test/mockApi.js')).mockApi(await importOriginal(), {
        updateSettings: vi.fn().mockResolvedValue({}),
        updatePushToConfig: vi.fn().mockResolvedValue({}),
    }))

describe('useTelescopeSetup', () => {
    let settingsRef
    let connectedCameraInfoRef
    let refreshSettings
    let applySettings

    beforeEach(() => {
        vi.clearAllMocks()
        vi.useFakeTimers()
        
        settingsRef = ref(null)
        connectedCameraInfoRef = ref(null)
        refreshSettings = vi.fn().mockResolvedValue({})
        // The settings store: posts, then adopts an answer that carries settings (the
        // mocked `{}` does not).
        applySettings = vi.fn(async (patch) => {
            const answer = await api.updateSettings(patch)
            if (answer?.telescope) settingsRef.value = answer
            return answer
        })
    })

    afterEach(() => {
        vi.useRealTimers()
    })

    function setupComposable(withErrorHandling = async (fn) => { await fn() }) {
        let result
        const TestComponent = {
            setup() {
                result = useTelescopeSetup({
                    connectedCameraInfo: connectedCameraInfoRef,
                    withErrorHandling,
                })
                return () => {}
            }
        }
        mount(TestComponent, {
            global: {
                provide: contextProvides({
                    'settings': settingsRef,
                    refreshSettings,
                    applySettings,
                })
            }
        })
        return result
    }

    it('should initialize with null values', () => {
        const {focalLength, pixelSizeX} = setupComposable()
        expect(focalLength.value).toBeNull()
        expect(pixelSizeX.value).toBeNull()
    })

    it('should restore focal length from settings', async () => {
        const {focalLength} = setupComposable()
        
        settingsRef.value = {
            telescope: { focal_length_mm: 500 }
        }
        await nextTick()
        await flushPromises()

        expect(focalLength.value).toBe(500)
    })

    it('should not process connectedCameraInfo until settings are loaded (race condition fix)', async () => {
        const {focalLength, pixelSizeX} = setupComposable()

        // 1. Camera connects BEFORE settings are loaded
        connectedCameraInfoRef.value = {
            name: 'Test Camera',
            pixel_size_x_um: 3.76,
            max_width: 3000,
            max_height: 2000
        }
        await nextTick()
        await flushPromises()

        // Because settings haven't loaded (initialSyncDone is false), pixel size shouldn't be overwritten yet
        expect(pixelSizeX.value).toBeNull()

        // 2. Now settings finish loading
        settingsRef.value = {
            telescope: { focal_length_mm: 1000 },
            camera_telescope_profiles: {
                'Test Camera': {
                    focal_length_mm: 800,
                    pixel_size_x_um: 3.76,
                    pixel_size_y_um: 3.76,
                    sensor_width_px: 3000,
                    sensor_height_px: 2000,
                    barlow_coeff: 1.0
                }
            },
            last_camera_name: 'Test Camera'
        }
        await nextTick()
        await flushPromises()

        // Now both syncDone and newInfo are true, the watcher should trigger and restore the profile
        expect(focalLength.value).toBe(800)
        expect(pixelSizeX.value).toBe(3.76)
    })

    it('should inherit from last camera if no profile exists for new camera', async () => {
        const {focalLength, pixelSizeX} = setupComposable()

        // Settings load
        settingsRef.value = {
            telescope: { focal_length_mm: 1000 },
            camera_telescope_profiles: {
                'Old Camera': {
                    focal_length_mm: 1200,
                    pixel_size_x_um: 4.63
                }
            },
            last_camera_name: 'Old Camera'
        }
        await nextTick()
        await flushPromises()

        // Connect new camera
        connectedCameraInfoRef.value = {
            name: 'New Camera',
            pixel_size_x_um: 2.4,
            max_width: 1000,
            max_height: 1000
        }
        await nextTick()
        await flushPromises()

        // Should inherit focal length from last camera, but use pixel size from new camera
        expect(focalLength.value).toBe(1200)
        expect(pixelSizeX.value).toBe(2.4)
    })

    it('should save settings automatically when values change', async () => {
        const {focalLength} = setupComposable()

        // Load settings
        settingsRef.value = {
            telescope: {}
        }
        await nextTick()
        await flushPromises()

        // Change focal length
        focalLength.value = 750
        await nextTick()

        // Wait for debounce
        vi.runAllTimers()
        await nextTick()

        expect(api.updateSettings).toHaveBeenCalledWith(expect.objectContaining({
            telescope: expect.objectContaining({
                focal_length_mm: 750
            })
        }))
    })

    it('keeps each camera profile when switching back and forth', async () => {
        // The outgoing camera's profile was never saved: the callback destructured the
        // old-values array in its parameter list and then indexed it again, so the
        // previous camera name was always undefined. Switching away from a camera lost
        // whatever had been set for it since the last debounced save.
        const {focalLength, pixelSizeX} = setupComposable()

        settingsRef.value = {
            telescope: {},
            camera_telescope_profiles: {},
            last_camera_name: null,
        }
        await nextTick()
        await flushPromises()

        // Camera A, configured by hand.
        connectedCameraInfoRef.value = {name: 'Camera A', pixel_size_x_um: 3.76, max_width: 3000, max_height: 2000}
        await nextTick()
        await flushPromises()
        focalLength.value = 1200
        await nextTick()

        // Switch to camera B, which has a different sensor.
        connectedCameraInfoRef.value = {name: 'Camera B', pixel_size_x_um: 2.4, max_width: 1000, max_height: 1000}
        await nextTick()
        await flushPromises()
        focalLength.value = 400
        await nextTick()

        expect(pixelSizeX.value).toBe(2.4)

        // Back to A: its own focal length and pixel size must come back.
        connectedCameraInfoRef.value = {name: 'Camera A', pixel_size_x_um: 3.76, max_width: 3000, max_height: 2000}
        await nextTick()
        await flushPromises()

        expect(focalLength.value).toBe(1200)
        expect(pixelSizeX.value).toBe(3.76)
    })

    it('does not re-apply driver defaults when the camera list refreshes', async () => {
        // `connectedCameraInfo` is a computed over the camera list, so every
        // refreshCameras() hands the watcher a new object for the *same* camera. With
        // the same-camera guard broken, each of those re-ran the resolve block and
        // overwrote a manually entered pixel size with the driver's value.
        const {pixelSizeX} = setupComposable()

        settingsRef.value = {
            telescope: {},
            camera_telescope_profiles: {},
            last_camera_name: null,
        }
        await nextTick()
        await flushPromises()

        connectedCameraInfoRef.value = {name: 'Camera A', pixel_size_x_um: 3.76, max_width: 3000, max_height: 2000}
        await nextTick()
        await flushPromises()

        // The user overrides the pixel size by hand.
        pixelSizeX.value = 9.99
        await nextTick()

        // A camera list refresh: same camera, new object identity.
        connectedCameraInfoRef.value = {name: 'Camera A', pixel_size_x_um: 3.76, max_width: 3000, max_height: 2000}
        await nextTick()
        await flushPromises()

        expect(pixelSizeX.value).toBe(9.99)
    })

    it('does not push a computed FOV to the solver', async () => {
        // The backend derives the image-height FOV from the telescope block itself and
        // prefers a FOV measured by a previous solve. Posting max(fovX, fovY) here
        // overwrote that a few milliseconds later -- with the sensor *width* field, on
        // any non-square sensor.
        const {focalLength, pixelSizeX, pixelSizeY, sensorWidthPx, sensorHeightPx} = setupComposable()

        settingsRef.value = {telescope: {}}
        await nextTick()
        await flushPromises()

        focalLength.value = 1200
        pixelSizeX.value = 2.9
        pixelSizeY.value = 2.9
        sensorWidthPx.value = 2712
        sensorHeightPx.value = 1538
        await nextTick()

        vi.runAllTimers()
        await nextTick()
        await flushPromises()

        expect(api.updateSettings).toHaveBeenCalled()
        expect(api.updatePushToConfig).not.toHaveBeenCalled()
    })
    // Push-To mounts this after settings have loaded. Reading the stored values back in
    // must not count as an edit, or every first visit rewrote the telescope block.
    describe('mounting over loaded settings', () => {
        const telescope = {
            focal_length_mm: 500,
            pixel_size_x_um: 3.76,
            pixel_size_y_um: 3.76,
            sensor_width_px: 3000,
            sensor_height_px: 2000,
            barlow_coeff: 1,
        }
        const cameraA = {name: 'Camera A', pixel_size_x_um: 3.76, max_width: 3000, max_height: 2000}

        async function mountLoaded(cameraInfo) {
            settingsRef.value = {
                telescope,
                camera_telescope_profiles: {'Camera A': {...telescope}},
                last_camera_name: 'Camera A',
            }
            connectedCameraInfoRef.value = cameraInfo
            const result = setupComposable()
            await nextTick()
            await flushPromises()
            vi.runAllTimers()
            await flushPromises()
            return result
        }

        it('writes nothing without a camera', async () => {
            const {focalLength} = await mountLoaded(null)
            expect(focalLength.value).toBe(500)
            expect(api.updateSettings).not.toHaveBeenCalled()
        })

        it('writes nothing when the stored camera is the connected one', async () => {
            const {focalLength} = await mountLoaded(cameraA)
            expect(focalLength.value).toBe(500)
            expect(api.updateSettings).not.toHaveBeenCalled()
        })

        it('still saves a camera other than the stored one', async () => {
            await mountLoaded({name: 'Camera B', pixel_size_x_um: 2.4, max_width: 1000, max_height: 1000})
            expect(api.updateSettings).toHaveBeenCalledTimes(1)
            expect(api.updateSettings).toHaveBeenCalledWith(expect.objectContaining({
                last_camera_name: 'Camera B',
                telescope: expect.objectContaining({focal_length_mm: 500, pixel_size_x_um: 2.4}),
            }))
        })

        // A visited Push-To tab stays mounted while hidden, so a camera switched from the
        // Main tab is resolved here too: the outgoing profile is kept, the new one stored.
        it('saves both profiles when the camera changes after mounting', async () => {
            const {focalLength} = await mountLoaded(cameraA)
            focalLength.value = 900
            await nextTick()
            connectedCameraInfoRef.value = {name: 'Camera B', pixel_size_x_um: 2.4, max_width: 1000, max_height: 1000}
            await nextTick()
            vi.runAllTimers()
            await flushPromises()

            expect(api.updateSettings).toHaveBeenCalledTimes(1)
            const payload = api.updateSettings.mock.calls[0][0]
            expect(payload.last_camera_name).toBe('Camera B')
            expect(payload.camera_telescope_profiles['Camera A']).toMatchObject({focal_length_mm: 900, pixel_size_x_um: 3.76})
            expect(payload.camera_telescope_profiles['Camera B']).toMatchObject({focal_length_mm: 900, pixel_size_x_um: 2.4})
        })

        it('saves an edit made after mounting', async () => {
            const {focalLength} = await mountLoaded(cameraA)
            focalLength.value = 750
            await nextTick()
            vi.runAllTimers()
            await flushPromises()
            expect(api.updateSettings).toHaveBeenCalledTimes(1)
            expect(api.updateSettings).toHaveBeenCalledWith(expect.objectContaining({
                telescope: expect.objectContaining({focal_length_mm: 750}),
            }))
        })
    })
    // The Push-To tab stays mounted for the page's life, so the server's later changes
    // (a profile seeded at connect, another client's edit) must reach it.
    describe('following the server after mounting', () => {
        const mainScope = {
            focal_length_mm: 650,
            pixel_size_x_um: 3.76,
            pixel_size_y_um: 3.76,
            sensor_width_px: 3008,
            sensor_height_px: 3008,
            barlow_coeff: 1,
        }
        const guideScope = {
            focal_length_mm: 200,
            pixel_size_x_um: 2.9,
            pixel_size_y_um: 2.9,
            sensor_width_px: 1936,
            sensor_height_px: 1096,
            barlow_coeff: 1,
        }
        const cameraA = {name: 'Camera A', pixel_size_x_um: 3.76, max_width: 3008, max_height: 3008}
        const cameraB = {name: 'Camera B', pixel_size_x_um: 2.9, max_width: 1936, max_height: 1096}

        function serverSettings({profiles = {'Camera A': mainScope}, telescope = mainScope, last = 'Camera A'} = {}) {
            return {telescope, camera_telescope_profiles: profiles, last_camera_name: last}
        }

        async function settle() {
            await nextTick()
            await flushPromises()
        }

        async function mountWith(cameraInfo, settings = serverSettings(), withErrorHandling) {
            settingsRef.value = settings
            connectedCameraInfoRef.value = cameraInfo
            const result = withErrorHandling ? setupComposable(withErrorHandling) : setupComposable()
            await settle()
            vi.runAllTimers()
            await flushPromises()
            api.updateSettings.mockClear()
            return result
        }

        async function swallowErrors(fn) {
            try {
                await fn()
            } catch {
                // useError shows it; the edit must stay on screen
            }
        }

        async function runSaves() {
            vi.runAllTimers()
            await flushPromises()
            return api.updateSettings.mock.calls.map((call) => call[0])
        }

        /** The profiles the server holds after `payloads`: it upserts the ones each names. */
        function stored(payloads) {
            return payloads.reduce(
                (profiles, payload) => ({...profiles, ...payload.camera_telescope_profiles}),
                {...settingsRef.value.camera_telescope_profiles},
            )
        }

        /** Every value `payloads` posted for `name`'s profile. */
        function postedFor(payloads, name) {
            return payloads.map((payload) => payload.camera_telescope_profiles?.[name]).filter(Boolean)
        }

        it('restores a profile the server seeded for a camera connected later', async () => {
            const {focalLength} = await mountWith(cameraA)

            settingsRef.value = serverSettings({profiles: {'Camera A': mainScope, 'Camera B': guideScope}})
            await settle()
            connectedCameraInfoRef.value = cameraB
            await settle()

            expect(focalLength.value).toBe(200)
            const profiles = stored(await runSaves())
            expect(profiles['Camera B']).toMatchObject({focal_length_mm: 200})
            expect(profiles['Camera A']).toMatchObject({focal_length_mm: 650})
        })

        // The camera list and the settings refresh after a connect race each other.
        it('takes the seeded profile when it lands after the camera was selected', async () => {
            const {focalLength} = await mountWith(cameraA)

            connectedCameraInfoRef.value = cameraB
            await settle()
            expect(focalLength.value).toBe(650) // inherited, nothing better known yet

            settingsRef.value = serverSettings({profiles: {'Camera A': mainScope, 'Camera B': guideScope}})
            await settle()

            expect(focalLength.value).toBe(200)
            const payloads = await runSaves()
            expect(payloads).toHaveLength(1)
            expect(payloads[0].telescope).toMatchObject({focal_length_mm: 200})
            expect(stored(payloads)['Camera B']).toMatchObject({focal_length_mm: 200})
        })

        it('shows a focal length another client stored, without writing it back', async () => {
            const {focalLength} = await mountWith(cameraA)

            const edited = {...mainScope, focal_length_mm: 1000}
            settingsRef.value = serverSettings({telescope: edited, profiles: {'Camera A': edited}})
            await settle()

            expect(focalLength.value).toBe(1000)
            expect(await runSaves()).toEqual([])
        })

        it('follows the flat block while no camera is connected', async () => {
            const {focalLength} = await mountWith(null)

            // As a client without a camera saves it: the flat block and its last camera.
            const edited = {...mainScope, focal_length_mm: 1200}
            settingsRef.value = serverSettings({telescope: edited, profiles: {'Camera A': edited}})
            await settle()

            expect(focalLength.value).toBe(1200)
            expect(await runSaves()).toEqual([])
        })

        it('keeps a typed focal length through a sync inside the save delay', async () => {
            const {focalLength} = await mountWith(cameraA)
            focalLength.value = 750
            await nextTick()

            settingsRef.value = serverSettings({profiles: {'Camera A': mainScope, 'Camera B': guideScope}})
            await settle()

            expect(focalLength.value).toBe(750)
            const payloads = await runSaves()
            expect(payloads).toHaveLength(1)
            expect(stored(payloads)['Camera A']).toMatchObject({focal_length_mm: 750})
            expect(stored(payloads)['Camera B']).toMatchObject({focal_length_mm: 200})
        })

        // Review of 4fc5053, R2: the server upserts what a save names, so a profile this
        // page last fetched (or never saw, like a seed from a connect) is not resent.
        it('names only the profile it changed', async () => {
            const {focalLength} = await mountWith(cameraA, serverSettings({
                profiles: {'Camera A': mainScope, 'Camera B': guideScope},
            }))
            focalLength.value = 750
            await nextTick()

            const payloads = await runSaves()
            expect(payloads).toHaveLength(1)
            expect(Object.keys(payloads[0].camera_telescope_profiles)).toEqual(['Camera A'])
        })

        it('sends a profile whose save was refused with the next save', async () => {
            const {focalLength} = await mountWith(cameraA, serverSettings({
                profiles: {'Camera A': mainScope, 'Camera B': guideScope},
            }), swallowErrors)
            api.updateSettings.mockRejectedValueOnce(new Error('offline'))
            focalLength.value = 750
            await nextTick()
            await runSaves()

            connectedCameraInfoRef.value = cameraB
            await settle()

            const payloads = await runSaves()
            expect(payloads.at(-1).camera_telescope_profiles['Camera A']).toMatchObject({focal_length_mm: 750})
        })

        it('keeps an edit parked by a camera switch through a sync', async () => {
            const {focalLength} = await mountWith(cameraA, serverSettings({
                profiles: {'Camera A': mainScope, 'Camera B': guideScope},
            }))
            focalLength.value = 900
            await nextTick()
            connectedCameraInfoRef.value = cameraB
            await settle()

            settingsRef.value = serverSettings({profiles: {'Camera A': mainScope, 'Camera B': guideScope}})
            await settle()

            const payloads = await runSaves()
            expect(payloads).toHaveLength(1)
            expect(stored(payloads)['Camera A']).toMatchObject({focal_length_mm: 900})
            expect(stored(payloads)['Camera B']).toMatchObject({focal_length_mm: 200})
        })

        it('keeps an edit whose save was refused through the next sync', async () => {
            const {focalLength} = await mountWith(cameraA, serverSettings(), swallowErrors)
            api.updateSettings.mockRejectedValueOnce(new Error('offline'))
            focalLength.value = 750
            await nextTick()
            await runSaves()

            settingsRef.value = serverSettings()
            await settle()

            expect(focalLength.value).toBe(750)
        })

        // The solver's rig key tells an unset Barlow from 1x, so 1 must not be written
        // over it just because the field shows 1.
        it('writes nothing over an unset Barlow on mount', async () => {
            const unset = {...mainScope, barlow_coeff: null}
            settingsRef.value = serverSettings({telescope: unset, profiles: {'Camera A': unset}})
            connectedCameraInfoRef.value = cameraA
            setupComposable()
            await settle()

            expect(await runSaves()).toEqual([])
        })

        // A refresh fetched before the save landed (another panel's write, the cooler's
        // own refresh) carries the old value; it must not undo the edit being saved.
        async function typeWhileSaveIsInFlight(save, withErrorHandling) {
            const composable = await mountWith(cameraA, serverSettings(), withErrorHandling)
            api.updateSettings.mockImplementationOnce(() => new Promise((resolve, reject) => {
                save.resolve = resolve
                save.reject = reject
            }))
            composable.focalLength.value = 750
            await nextTick()
            vi.advanceTimersByTime(SETTINGS_DEBOUNCE_MS)
            await flushPromises()
            expect(api.updateSettings).toHaveBeenCalledTimes(1)

            settingsRef.value = serverSettings()
            await settle()
            return composable
        }

        it('keeps an edit through a stale sync while its save is in flight', async () => {
            const save = {}
            const {focalLength} = await typeWhileSaveIsInFlight(save)
            expect(focalLength.value).toBe(750)

            const saved = {...mainScope, focal_length_mm: 750}
            save.resolve(serverSettings({telescope: saved, profiles: {'Camera A': saved}}))
            await flushPromises()
            vi.runAllTimers()
            await flushPromises()

            expect(focalLength.value).toBe(750)
            expect(api.updateSettings).toHaveBeenCalledTimes(1)
            // The held settings predate the save; the store adopted its answer instead of
            // reading them again.
            expect(refreshSettings).not.toHaveBeenCalled()
        })

        // Adopted while the save was in flight, and newer than it: the panel follows it.
        it('follows what the store holds once a landed save releases a held sync', async () => {
            const save = {}
            const composable = await typeWhileSaveIsInFlight(save)

            const saved = {...mainScope, focal_length_mm: 750}
            save.resolve(serverSettings({
                telescope: saved,
                profiles: {'Camera A': saved, 'Camera B': guideScope},
            }))
            await flushPromises()
            connectedCameraInfoRef.value = cameraB
            await settle()

            expect(composable.focalLength.value).toBe(200)
            expect(refreshSettings).not.toHaveBeenCalled()
        })

        it('keeps an edit through a stale sync when its save is then refused', async () => {
            const save = {}
            const {focalLength} = await typeWhileSaveIsInFlight(save, swallowErrors)

            save.reject(new Error('offline'))
            await flushPromises()

            expect(focalLength.value).toBe(750)
            expect(refreshSettings).not.toHaveBeenCalled()
            focalLength.value = 760
            await nextTick()
            const payloads = await runSaves()
            expect(payloads.at(-1).telescope).toMatchObject({focal_length_mm: 760})
        })

        // What `ensure_camera_telescope_profile` really seeds for a camera on another
        // sensor: no focal length, since the flat block's belongs to a different scope.
        const seededGuide = {...guideScope, focal_length_mm: null}

        it('does not post the outgoing focal length over a seeded profile without one', async () => {
            const {focalLength} = await mountWith(cameraA)

            settingsRef.value = serverSettings({profiles: {'Camera A': mainScope, 'Camera B': seededGuide}})
            await settle()
            connectedCameraInfoRef.value = cameraB
            await settle()

            const payloads = await runSaves()
            expect(postedFor(payloads, 'Camera B').map((profile) => profile.focal_length_mm)).not.toContain(650)
            expect(stored(payloads)['Camera B'].focal_length_mm).toBeNull()
            expect(focalLength.value).toBeNull()
        })

        it('does not post the outgoing focal length when the focal-less seed lands late', async () => {
            const {focalLength} = await mountWith(cameraA)

            connectedCameraInfoRef.value = cameraB
            await settle()
            settingsRef.value = serverSettings({profiles: {'Camera A': mainScope, 'Camera B': seededGuide}})
            await settle()

            const payloads = await runSaves()
            expect(postedFor(payloads, 'Camera B').map((profile) => profile.focal_length_mm)).not.toContain(650)
            expect(stored(payloads)['Camera B'].focal_length_mm).toBeNull()
            expect(focalLength.value).toBeNull()
        })

        // Review of 4fc5053, R3: the field shows an unset Barlow as 1x, and the outgoing
        // camera's profile is rebuilt from the fields on a switch. The server and Pro's
        // rig key tell None from 1x, so the click restarts the solve and orphans the
        // FOV measured under the old key.
        it('keeps an unset Barlow unset when the camera switches away', async () => {
            const unset = {...mainScope, barlow_coeff: null}
            await mountWith(cameraA, serverSettings({
                telescope: unset,
                profiles: {'Camera A': unset, 'Camera B': guideScope},
            }))

            connectedCameraInfoRef.value = cameraB
            await settle()

            const payloads = await runSaves()
            expect(payloads).toHaveLength(1)
            expect(postedFor(payloads, 'Camera A').map((profile) => profile.barlow_coeff)).not.toContain(1)
            expect(stored(payloads)['Camera A'].barlow_coeff).toBeNull()
        })
    })
})
