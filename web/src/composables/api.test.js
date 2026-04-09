import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {
    startCapture,
    stopCapture,
    getCaptureStatus,
    getSettings,
    updateSettings,
    listCameras,
    getCameraInfo,
    connectCamera,
    disconnectCamera,
} from './api.js'

describe('API Client', () => {
    let fetchMock

    beforeEach(() => {
        fetchMock = vi.fn()
        global.fetch = fetchMock
    })

    afterEach(() => {
        vi.restoreAllMocks()
    })

    // Helper to create successful API response
    function mockSuccess(data) {
        return Promise.resolve({
            text: () => Promise.resolve(JSON.stringify({success: true, data})),
        })
    }

    // Helper to create error API response
    function mockError(error) {
        return Promise.resolve({
            text: () => Promise.resolve(JSON.stringify({success: false, error})),
        })
    }

    describe('Capture Control', () => {
        it('startCapture sends POST to /api/capture/start', async () => {
            const response = {message: 'Capture started', camera_id: 'cam1'}
            fetchMock.mockReturnValue(mockSuccess(response))

            const result = await startCapture('cam1')

            expect(fetchMock).toHaveBeenCalledWith('/api/capture/start', {
                method: 'POST',
                cache: 'no-store',
                headers: {'Content-Type': 'application/json'},
                body: JSON.stringify({camera_id: 'cam1'}),
            })
            expect(result).toEqual(response)
        })

        it('startCapture without camera_id sends empty body', async () => {
            fetchMock.mockReturnValue(mockSuccess({message: 'Capture started'}))

            await startCapture()

            expect(fetchMock).toHaveBeenCalledWith('/api/capture/start', {
                method: 'POST',
                cache: 'no-store',
                headers: {'Content-Type': 'application/json'},
                body: JSON.stringify({}),
            })
        })

        it('stopCapture sends POST to /api/capture/stop', async () => {
            const response = {message: 'Capture stopping'}
            fetchMock.mockReturnValue(mockSuccess(response))

            const result = await stopCapture()

            expect(fetchMock).toHaveBeenCalledWith('/api/capture/stop', {
                method: 'POST',
                cache: 'no-store',
                headers: {'Content-Type': 'application/json'},
            })
            expect(result).toEqual(response)
        })

        it('getCaptureStatus sends GET to /api/capture/status', async () => {
            const status = {
                state: 'Capturing',
                frame_count: 42,
                stacked_count: 40,
                rejected_count: 2,
            }
            fetchMock.mockReturnValue(mockSuccess(status))

            const result = await getCaptureStatus()

            expect(fetchMock).toHaveBeenCalledWith('/api/capture/status', {
                cache: 'no-store',
                headers: {'Content-Type': 'application/json'},
            })
            expect(result).toEqual(status)
        })
    })

    describe('Settings', () => {
        it('getSettings sends GET to /api/settings', async () => {
            const settings = {
                exposure_us: 1000000,
                gain: 100,
                auto_stretch: true,
            }
            fetchMock.mockReturnValue(mockSuccess(settings))

            const result = await getSettings()

            expect(fetchMock).toHaveBeenCalledWith('/api/settings', {
                cache: 'no-store',
                headers: {'Content-Type': 'application/json'},
            })
            expect(result).toEqual(settings)
        })

        it('updateSettings sends POST with partial settings', async () => {
            const updatedSettings = {exposure_us: 2000000, gain: 150}
            fetchMock.mockReturnValue(mockSuccess(updatedSettings))

            const result = await updateSettings({exposure_us: 2000000})

            expect(fetchMock).toHaveBeenCalledWith('/api/settings', {
                method: 'POST',
                cache: 'no-store',
                headers: {'Content-Type': 'application/json'},
                body: JSON.stringify({exposure_us: 2000000}),
            })
            expect(result).toEqual(updatedSettings)
        })

        it('updateSettings can update multiple fields', async () => {
            const updates = {
                exposure_us: 5000000,
                gain: 200,
                stacking: true,
                rejection_sigma: 2.5,
            }
            fetchMock.mockReturnValue(mockSuccess(updates))

            await updateSettings(updates)

            expect(fetchMock).toHaveBeenCalledWith('/api/settings', {
                method: 'POST',
                cache: 'no-store',
                headers: {'Content-Type': 'application/json'},
                body: JSON.stringify(updates),
            })
        })
    })

    describe('Cameras', () => {
        it('listCameras sends GET to /api/cameras', async () => {
            const cameras = [
                {id: 'playerone_0', name: 'Neptune-C II', connected: true},
                {id: 'playerone_1', name: 'Mars-M', connected: false},
            ]
            fetchMock.mockReturnValue(mockSuccess(cameras))

            const result = await listCameras()

            expect(fetchMock).toHaveBeenCalledWith('/api/cameras', {
                cache: 'no-store',
                headers: {'Content-Type': 'application/json'},
            })
            expect(result).toEqual(cameras)
        })

        it('getCameraInfo sends GET to /api/cameras/:id', async () => {
            const cameraInfo = {
                id: 'playerone_0',
                name: 'Neptune-C II',
                max_width: 2712,
                max_height: 1538,
            }
            fetchMock.mockReturnValue(mockSuccess(cameraInfo))

            const result = await getCameraInfo('playerone_0')

            expect(fetchMock).toHaveBeenCalledWith('/api/cameras/playerone_0', {
                cache: 'no-store',
                headers: {'Content-Type': 'application/json'},
            })
            expect(result).toEqual(cameraInfo)
        })

        it('getCameraInfo URL-encodes camera ID', async () => {
            fetchMock.mockReturnValue(mockSuccess({}))

            await getCameraInfo('camera/with/slashes')

            expect(fetchMock).toHaveBeenCalledWith(
                '/api/cameras/camera%2Fwith%2Fslashes',
                expect.any(Object)
            )
        })

        it('connectCamera sends POST to /api/cameras/:id/connect', async () => {
            const response = {message: 'Camera connected', camera_id: 'playerone_0'}
            fetchMock.mockReturnValue(mockSuccess(response))

            const result = await connectCamera('playerone_0')

            expect(fetchMock).toHaveBeenCalledWith('/api/cameras/playerone_0/connect', {
                method: 'POST',
                cache: 'no-store',
                headers: {'Content-Type': 'application/json'},
            })
            expect(result).toEqual(response)
        })

        it('disconnectCamera sends POST to /api/cameras/:id/disconnect', async () => {
            const response = {message: 'Camera disconnected'}
            fetchMock.mockReturnValue(mockSuccess(response))

            const result = await disconnectCamera('playerone_0')

            expect(fetchMock).toHaveBeenCalledWith('/api/cameras/playerone_0/disconnect', {
                method: 'POST',
                cache: 'no-store',
                headers: {'Content-Type': 'application/json'},
            })
            expect(result).toEqual(response)
        })
    })

    describe('Error Handling', () => {
        it('throws error when API returns success: false', async () => {
            fetchMock.mockReturnValue(mockError('Camera not found'))

            await expect(getCameraInfo('nonexistent')).rejects.toThrow('Camera not found')
        })

        it('throws generic error when no error message provided', async () => {
            fetchMock.mockReturnValue(
                Promise.resolve({
                    text: () => Promise.resolve(JSON.stringify({success: false})),
                })
            )

            await expect(getSettings()).rejects.toThrow('API request failed')
        })

        it('throws user-friendly error on network failure', async () => {
            fetchMock.mockRejectedValue(new Error('Network error'))

            await expect(listCameras()).rejects.toThrow(
                'Server unavailable. Please ensure the server is running.'
            )
        })

        it('throws error on empty response', async () => {
            fetchMock.mockReturnValue(
                Promise.resolve({
                    text: () => Promise.resolve(''),
                })
            )

            await expect(getSettings()).rejects.toThrow(
                'Server unavailable. Please ensure the server is running.'
            )
        })

        it('throws error on invalid JSON response', async () => {
            fetchMock.mockReturnValue(
                Promise.resolve({
                    text: () => Promise.resolve('not valid json'),
                })
            )

            await expect(getSettings()).rejects.toThrow(
                'Server returned invalid response. Please ensure the server is running.'
            )
        })
    })
})
