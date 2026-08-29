import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {ref, nextTick} from 'vue'
import {mount, flushPromises} from '@vue/test-utils'
import {useTelescopeSetup} from '../useTelescopeSetup.js'
import * as api from '../api.js'

// Mock API
vi.mock('../api.js', () => ({
    updateSettings: vi.fn().mockResolvedValue({}),
    updatePushToConfig: vi.fn().mockResolvedValue({}),
}))

describe('useTelescopeSetup', () => {
    let settingsRef
    let connectedCameraInfoRef

    beforeEach(() => {
        vi.clearAllMocks()
        vi.useFakeTimers()
        
        settingsRef = ref(null)
        connectedCameraInfoRef = ref(null)
    })

    afterEach(() => {
        vi.useRealTimers()
    })

    function setupComposable() {
        let result
        const TestComponent = {
            setup() {
                result = useTelescopeSetup({
                    connectedCameraInfo: connectedCameraInfoRef,
                    withErrorHandling: async (fn) => { await fn() }
                })
                return () => {}
            }
        }
        mount(TestComponent, {
            global: {
                provide: {
                    'settings': settingsRef
                }
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
})
