import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'

vi.mock('../api.js', async (importOriginal) =>
    (await import('../../test/mockApi.js')).mockApi(await importOriginal(), {
        getSettings: vi.fn(),
        listCameras: vi.fn(),
        getCapabilities: vi.fn(),
    }))

import {getCapabilities, getSettings, listCameras} from '../api.js'
import {camera, proCapabilities, settings} from '../../test/fixtures.js'

// The composable keeps module-level state; each test gets a fresh copy.
async function freshAppState() {
    vi.resetModules()
    const {useAppState} = await import('../useAppState.js')
    return useAppState()
}

const guide = (phase, extra = {}) => ({
    id: 'playerone_1',
    name: 'Neptune-C II',
    connected: true,
    role: 'guide',
    phase,
    ...extra,
})

describe('useAppState camera phases', () => {
    beforeEach(() => {
        listCameras.mockReset()
    })

    // 2026-09-20: a page opened while the guide loop ran had no phase for it and offered
    // "Start guide"; the list now carries the phase.
    it('takes each connected camera’s phase from the camera list', async () => {
        const app = await freshAppState()
        listCameras.mockResolvedValue([guide('guiding'), {id: 'x', name: 'Idle cam', connected: false}])

        await app.refreshCameras()

        expect(app.cameraPhase.value).toEqual({'Neptune-C II': 'guiding'})
    })

    it('keeps a phase an event delivered while the list was in flight', async () => {
        const app = await freshAppState()
        let answer
        listCameras.mockReturnValue(new Promise((resolve) => (answer = resolve)))

        const refreshing = app.refreshCameras()
        app.updateCameraPhase('Neptune-C II', 'guiding')
        answer([guide('idle')])
        await refreshing

        expect(app.cameraPhase.value['Neptune-C II']).toBe('guiding')
    })

    it('records when a warm-up is cut short at the latest', async () => {
        const app = await freshAppState()
        listCameras.mockResolvedValue([guide('warming_up', {warmup_remaining_s: 120})])

        const before = Date.now()
        await app.refreshCameras()

        const endsAt = app.warmupEndsAt.value['Neptune-C II']
        expect(endsAt).toBeGreaterThanOrEqual(before + 120_000)
        expect(endsAt).toBeLessThanOrEqual(Date.now() + 120_000)
    })

    it('lets the server’s snapshot replace every phase it holds', async () => {
        const app = await freshAppState()
        app.updateCameraPhase('Gone camera', 'idle')
        app.updateCameraPhase('Neptune-C II', 'idle')

        app.replaceCameraPhases([
            {name: 'Neptune-C II', role: 'guide', phase: 'warming_up', warmup_remaining_s: 60},
        ])

        expect(app.cameraPhase.value).toEqual({'Neptune-C II': 'warming_up'})
        expect(app.warmupEndsAt.value['Neptune-C II']).toBeGreaterThan(Date.now())
    })

    // Every page counts a warm-up down, not only the one whose Disconnect started it.
    it('takes the warm-up deadline from the phase event that announces it', async () => {
        const app = await freshAppState()

        const before = Date.now()
        app.updateCameraPhase('Neptune-C II', 'warming_up', 300)

        const endsAt = app.warmupEndsAt.value['Neptune-C II']
        expect(endsAt).toBeGreaterThanOrEqual(before + 300_000)
        expect(endsAt).toBeLessThanOrEqual(Date.now() + 300_000)
    })

    it('keeps a known deadline when a warm-up event carries none', async () => {
        const app = await freshAppState()
        app.updateCameraPhase('Neptune-C II', 'warming_up', 300)
        const endsAt = app.warmupEndsAt.value['Neptune-C II']

        app.updateCameraPhase('Neptune-C II', 'warming_up')

        expect(app.warmupEndsAt.value['Neptune-C II']).toBe(endsAt)
    })

    it('forgets the warm-up deadline once the camera leaves the warm-up', async () => {
        const app = await freshAppState()
        app.replaceCameraPhases([{name: 'Neptune-C II', role: 'guide', phase: 'warming_up', warmup_remaining_s: 60}])

        app.updateCameraPhase('Neptune-C II', 'disconnected')

        expect(app.cameraPhase.value).toEqual({})
        expect(app.warmupEndsAt.value).toEqual({})
    })
})

describe('useAppState loading', () => {
    beforeEach(() => {
        vi.useFakeTimers()
        getSettings.mockReset().mockResolvedValue(settings({use_simulated_camera: true}))
        listCameras.mockReset().mockResolvedValue([])
        getCapabilities.mockReset().mockResolvedValue(proCapabilities())
    })

    afterEach(() => {
        vi.useRealTimers()
    })

    it('loads settings, cameras and capabilities, following the simulator switch', async () => {
        const app = await freshAppState()

        await app.initializeState()

        expect(app.loading.value).toBe(false)
        expect(app.globalError.value).toBeNull()
        expect(app.settings.value.use_simulated_camera).toBe(true)
        expect(app.simulatorEnabled.value).toBe(true)
        expect(app.capabilities.value.has_pro).toBe(true)
    })

    it('reports a failed load and retries on its own', async () => {
        vi.spyOn(console, 'error').mockImplementation(() => {})
        getSettings.mockRejectedValueOnce(new Error('Server unavailable'))
        const app = await freshAppState()

        await app.initializeState()
        expect(app.globalError.value).toBe('Server unavailable')

        await vi.advanceTimersByTimeAsync(2000)
        expect(getSettings).toHaveBeenCalledTimes(2)
        expect(app.globalError.value).toBeNull()
        vi.restoreAllMocks()
    })

    it('rethrows a failed camera or capability load to its caller', async () => {
        vi.spyOn(console, 'error').mockImplementation(() => {})
        const app = await freshAppState()
        listCameras.mockRejectedValueOnce(new Error('cameras'))
        getCapabilities.mockRejectedValueOnce(new Error('capabilities'))

        await expect(app.refreshCameras()).rejects.toThrow('cameras')
        await expect(app.refreshCapabilities()).rejects.toThrow('capabilities')
        vi.restoreAllMocks()
    })

    it('sets and clears the global error', async () => {
        const app = await freshAppState()
        app.setGlobalError('Lost the camera')
        expect(app.globalError.value).toBe('Lost the camera')
        app.clearGlobalError()
        expect(app.globalError.value).toBeNull()
    })
})

describe('useAppState cameras', () => {
    const main = camera({id: 'zwo_0', name: 'ASI533MC Pro', role: 'main'})
    const guideCam = camera({id: 'qhy_1', name: 'QHY5III462C', role: 'guide'})
    const sim = camera({id: 'sim_0', name: 'Simulator', provider: 'Simulator', connected: false, role: null})

    beforeEach(() => {
        listCameras.mockReset()
    })

    it('selects the imaging camera, never the guide, when nothing is selected', async () => {
        const app = await freshAppState()
        listCameras.mockResolvedValue([guideCam, main])

        await app.refreshCameras()

        expect(app.selectedCamera.value.id).toBe('zwo_0')
        expect(app.selectedCameraRole.value).toBe('main')
        expect(app.mainCamera.value.id).toBe('zwo_0')
        expect(app.guideCamera.value.id).toBe('qhy_1')
        expect(app.hasGuideCamera.value).toBe(true)
        expect(app.connectedCameras.value).toHaveLength(2)
        expect(app.availableCameras.value).toHaveLength(0)
    })

    it('keeps a deliberate guide selection and drops one whose camera went away', async () => {
        const app = await freshAppState()
        listCameras.mockResolvedValue([main, guideCam])
        await app.refreshCameras()

        app.selectCamera('qhy_1')
        expect(app.selectedCameraRole.value).toBe('guide')

        listCameras.mockResolvedValue([main])
        await app.refreshCameras()
        expect(app.selectedCamera.value.id).toBe('zwo_0')
    })

    it('knows the simulator, and adds a discovered camera once', async () => {
        const app = await freshAppState()
        listCameras.mockResolvedValue([])
        await app.refreshCameras()

        app.addDiscoveredCamera(sim)
        app.addDiscoveredCamera(sim)
        expect(app.cameras.value).toHaveLength(1)
        expect(app.availableCameras.value).toHaveLength(1)

        app.selectCamera('sim_0')
        expect(app.isSimulatorCamera.value).toBe(true)
    })

    it('keeps the latest live status per camera', async () => {
        const app = await freshAppState()
        app.updateCameraStatus('ASI533MC Pro', {temperature_c: -9.5})
        app.updateCameraStatus('QHY5III462C', {temperature_c: 20})
        expect(app.cameraStatus.value['ASI533MC Pro'].temperature_c).toBe(-9.5)
        expect(Object.keys(app.cameraStatus.value)).toHaveLength(2)
    })

    it('drops the phase of a camera that disconnected', async () => {
        const app = await freshAppState()
        app.updateCameraPhase('ASI533MC Pro', 'capturing')
        app.updateCameraPhase('ASI533MC Pro', 'disconnected')
        expect(app.cameraPhase.value).toEqual({})
    })

    it('lets the simulator switch be set directly', async () => {
        const app = await freshAppState()
        app.setSimulatorEnabled(true)
        expect(app.simulatorEnabled.value).toBe(true)
    })
})

