import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'

vi.mock('../api.js', async (importOriginal) =>
    (await import('../../test/mockApi.js')).mockApi(await importOriginal(), {
        getSettings: vi.fn(),
        updateSettings: vi.fn(),
        listCameras: vi.fn(),
        getCapabilities: vi.fn(),
    }))

import {effectScope, nextTick} from 'vue'
import type {EffectScope} from 'vue'
import type {Camera, CameraPhase, Settings} from '../api.types.js'
import type {CameraStatus} from '../cameraStore.js'
import type {EventStream} from '../useEventStream.js'
import type {AppState} from '../useAppState.js'
import {getCapabilities, getSettings, listCameras, updateSettings} from '../api.js'
import {camera, proCapabilities, settings} from '../../test/fixtures.js'
import {pending} from '../../test/http.js'
import {createEventStreamStub, emitEvent, emitSync} from '../../test/eventStream.js'
import {__resetForTests, RETRY_INTERVAL_MS, useAppState} from '../useAppState.js'

// The state is a module default; each test gets a fresh one.
async function freshAppState() {
    __resetForTests()
    return useAppState()
}

const guide = (phase: CameraPhase, extra: Partial<Camera> = {}) => camera({
    id: 'playerone_1',
    name: 'Neptune-C II',
    connected: true,
    role: 'guide',
    phase,
    ...extra,
})

/** A cooled camera's status sample at `temperature_c`. */
const status = (temperature_c: number): CameraStatus => ({
    temperature_c, cooler_power: null, cooler_on: true, dew_heater_on: false, target_temp_c: null,
})


describe('useAppState camera phases', () => {
    beforeEach(() => {
        vi.mocked(listCameras).mockReset()
    })

    // 2026-09-20: a page opened while the guide loop ran had no phase for it and offered
    // "Start guide"; the list now carries the phase.
    it('takes each connected camera’s phase from the camera list', async () => {
        const app = await freshAppState()
        vi.mocked(listCameras).mockResolvedValue([guide('guiding'), camera({id: 'x', name: 'Idle cam', connected: false, role: null})])

        await app.refreshCameras()

        expect(app.cameraPhase.value).toEqual({'Neptune-C II': 'guiding'})
    })

    it('keeps a phase an event delivered while the list was in flight', async () => {
        const app = await freshAppState()
        const list = pending<Camera[]>()
        vi.mocked(listCameras).mockReturnValue(list.promise)

        const refreshing = app.refreshCameras()
        app.updateCameraPhase('Neptune-C II', 'guiding')
        list.resolve([guide('idle')])
        await refreshing

        expect(app.cameraPhase.value['Neptune-C II']).toBe('guiding')
    })

    it('records when a warm-up is cut short at the latest', async () => {
        const app = await freshAppState()
        vi.mocked(listCameras).mockResolvedValue([guide('warming_up', {warmup_remaining_s: 120})])

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
        vi.mocked(getSettings).mockReset().mockResolvedValue(settings({use_simulated_camera: true}))
        vi.mocked(listCameras).mockReset().mockResolvedValue([])
        vi.mocked(getCapabilities).mockReset().mockResolvedValue(proCapabilities())
    })

    afterEach(() => {
        vi.useRealTimers()
    })

    it('loads settings, cameras and capabilities, following the simulator switch', async () => {
        const app = await freshAppState()

        await app.initializeState()

        expect(app.loading.value).toBe(false)
        expect(app.globalError.value).toBeNull()
        expect(app.settings.value?.use_simulated_camera).toBe(true)
        expect(app.simulatorEnabled.value).toBe(true)
        expect(app.capabilities.value.has_pro).toBe(true)
    })

    it('reports a failed load and retries on its own', async () => {
        vi.spyOn(console, 'error').mockImplementation(() => {})
        vi.mocked(getSettings).mockRejectedValueOnce(new Error('Server unavailable'))
        const app = await freshAppState()

        await app.initializeState()
        expect(app.globalError.value).toBe('Server unavailable')

        await vi.advanceTimersByTimeAsync(RETRY_INTERVAL_MS)
        expect(getSettings).toHaveBeenCalledTimes(2)
        expect(app.globalError.value).toBeNull()
        vi.restoreAllMocks()
    })

    // A server that is down has the event socket down too; the sync its reconnect brings is the signal.
    it('retries a load that failed with the socket down on the first sync, not on a timer', async () => {
        vi.spyOn(console, 'error').mockImplementation(() => {})
        vi.mocked(getSettings).mockRejectedValueOnce(new Error('Server unavailable'))
        const app = await freshAppState()
        const stream = createEventStreamStub({connected: false})
        const scope = effectScope()
        scope.run(() => app.connectEvents(stream))

        await app.initializeState()
        await vi.advanceTimersByTimeAsync(10 * RETRY_INTERVAL_MS)
        expect(getSettings).toHaveBeenCalledTimes(1)

        stream.connected.value = true
        await nextTick()
        emitSync(stream)
        await vi.runOnlyPendingTimersAsync()
        expect(getSettings).toHaveBeenCalledTimes(2)
        expect(listCameras).toHaveBeenCalledTimes(2)
        expect(app.globalError.value).toBeNull()

        // Loaded: a later sync re-reads what no snapshot carries, never the capabilities.
        emitSync(stream)
        await vi.runOnlyPendingTimersAsync()
        expect(getSettings).toHaveBeenCalledTimes(3)
        expect(listCameras).toHaveBeenCalledTimes(3)
        expect(getCapabilities).toHaveBeenCalledTimes(2)
        scope.stop()
        vi.restoreAllMocks()
    })

    // A `settings_updated` sent while the socket was down (a phone asleep, another
    // client's edit, a server restart), or lost to a lag, reached no one, and kept-mounted
    // panels followed the old values.
    it('reads the settings again on every later sync, a lag\'s included', async () => {
        vi.mocked(getSettings)
            .mockResolvedValueOnce(settings({gain: 100}))
            .mockResolvedValueOnce(settings({gain: 300}))
            .mockResolvedValue(settings({gain: 500}))
        const app = await freshAppState()
        const stream = createEventStreamStub({connected: true})
        const scope = effectScope()
        scope.run(() => app.connectEvents(stream))
        await app.initializeState()
        emitSync(stream)

        emitSync(stream, {cause: 'lagged', dropped: 144})
        await vi.runOnlyPendingTimersAsync()
        expect(app.settings.value?.gain).toBe(300)

        emitSync(stream)
        await vi.runOnlyPendingTimersAsync()
        expect(getSettings).toHaveBeenCalledTimes(3)
        expect(app.settings.value?.gain).toBe(500)
        scope.stop()
    })

    it('reads nothing twice at startup for a first sync during or after the first load', async () => {
        for (const syncDuringLoad of [true, false]) {
            vi.mocked(getSettings).mockClear()
            vi.mocked(listCameras).mockClear()
            const app = await freshAppState()
            const stream = createEventStreamStub()
            const scope = effectScope()
            scope.run(() => app.connectEvents(stream))

            const loading = app.initializeState()
            if (syncDuringLoad) emitSync(stream)
            await loading
            if (!syncDuringLoad) emitSync(stream)
            await vi.runOnlyPendingTimersAsync()

            expect(getSettings, `sync during load: ${syncDuringLoad}`).toHaveBeenCalledTimes(1)
            expect(listCameras, `sync during load: ${syncDuringLoad}`).toHaveBeenCalledTimes(1)
            scope.stop()
        }
    })

    it('polls a server that keeps the socket up while refusing the load', async () => {
        vi.spyOn(console, 'error').mockImplementation(() => {})
        vi.mocked(getSettings).mockRejectedValueOnce(new Error('Internal error'))
        const app = await freshAppState()
        const scope = effectScope()
        scope.run(() => app.connectEvents(createEventStreamStub({connected: true})))

        await app.initializeState()
        await vi.advanceTimersByTimeAsync(RETRY_INTERVAL_MS)

        expect(getSettings).toHaveBeenCalledTimes(2)
        scope.stop()
        vi.restoreAllMocks()
    })

    it('rethrows a failed camera or capability load to its caller', async () => {
        vi.spyOn(console, 'error').mockImplementation(() => {})
        const app = await freshAppState()
        vi.mocked(listCameras).mockRejectedValueOnce(new Error('cameras'))
        vi.mocked(getCapabilities).mockRejectedValueOnce(new Error('capabilities'))

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
        vi.mocked(listCameras).mockReset()
    })

    it('selects the imaging camera, never the guide, when nothing is selected', async () => {
        const app = await freshAppState()
        vi.mocked(listCameras).mockResolvedValue([guideCam, main])

        await app.refreshCameras()

        expect(app.selectedCamera.value?.id).toBe('zwo_0')
        expect(app.selectedCameraRole.value).toBe('main')
        expect(app.mainCamera.value?.id).toBe('zwo_0')
        expect(app.guideCamera.value?.id).toBe('qhy_1')
        expect(app.hasGuideCamera.value).toBe(true)
        expect(app.connectedCameras.value).toHaveLength(2)
        expect(app.availableCameras.value).toHaveLength(0)
    })

    it('keeps a deliberate guide selection and drops one whose camera went away', async () => {
        const app = await freshAppState()
        vi.mocked(listCameras).mockResolvedValue([main, guideCam])
        await app.refreshCameras()

        app.selectCamera('qhy_1')
        expect(app.selectedCameraRole.value).toBe('guide')

        vi.mocked(listCameras).mockResolvedValue([main])
        await app.refreshCameras()
        expect(app.selectedCamera.value?.id).toBe('zwo_0')
    })

    it('knows the simulator, and adds a discovered camera once', async () => {
        const app = await freshAppState()
        vi.mocked(listCameras).mockResolvedValue([])
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
        app.updateCameraStatus('ASI533MC Pro', status(-9.5))
        app.updateCameraStatus('QHY5III462C', status(20))
        expect(app.cameraStatus.value['ASI533MC Pro']?.temperature_c).toBe(-9.5)
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


describe('useAppState events', () => {
    let app: AppState
    let stream: EventStream
    let scope: EffectScope

    beforeEach(async () => {
        vi.mocked(getSettings).mockReset().mockResolvedValue(settings())
        vi.mocked(listCameras).mockReset().mockResolvedValue([])
        vi.mocked(getCapabilities).mockReset().mockResolvedValue(proCapabilities())
        app = await freshAppState()
        stream = createEventStreamStub()
        scope = effectScope()
        scope.run(() => app.connectEvents(stream))
    })

    /** Past the first load, which fetches the list itself. */
    async function loaded() {
        await app.initializeState()
        vi.mocked(listCameras).mockClear()
    }

    afterEach(() => scope.stop())

    it('takes phases from the server snapshot without listing the cameras again', async () => {
        await loaded()

        emitEvent(stream, {type: 'camera_phases', cameras: [{name: 'Neptune-C II', role: 'guide', phase: 'guiding'}]})

        expect(app.cameraPhase.value).toEqual({'Neptune-C II': 'guiding'})
        expect(listCameras).not.toHaveBeenCalled()
    })

    // A page that was away, or lagged, also missed cameras connecting and disconnecting.
    it('lists the cameras again on a later sync, once for a burst', async () => {
        await loaded()
        emitSync(stream)
        expect(listCameras).not.toHaveBeenCalled()
        const list = pending<Camera[]>()
        vi.mocked(listCameras)
            .mockReturnValueOnce(list.promise)
            .mockResolvedValue([guide('guiding')])

        emitSync(stream, {cause: 'lagged', dropped: 144})
        emitSync(stream)
        expect(listCameras).toHaveBeenCalledTimes(1)
        list.resolve([])

        await vi.waitFor(() => expect(app.hasGuideCamera.value).toBe(true))
        expect(listCameras).toHaveBeenCalledTimes(2)
    })

    it('follows a camera’s live status, phase, disconnect and discovery', async () => {
        await loaded()
        emitEvent(stream, {type: 'camera_status_updated', name: 'Cam', temperature_c: -9.5, cooler_on: true, dew_heater_on: false})
        expect(app.cameraStatus.value.Cam).toEqual({
            temperature_c: -9.5, cooler_power: null, cooler_on: true, dew_heater_on: false, target_temp_c: null,
        })

        emitEvent(stream, {type: 'camera_phase_changed', name: 'Cam', phase: 'warming_up', warmup_remaining_s: 60})
        expect(app.cameraPhase.value.Cam).toBe('warming_up')
        expect(app.warmupEndsAt.value.Cam).toBeGreaterThan(Date.now())

        emitEvent(stream, {type: 'camera_disconnected', name: 'Cam'})
        expect(app.cameraPhase.value).toEqual({})
        expect(listCameras).toHaveBeenCalledTimes(1)

        emitEvent(stream, {type: 'camera_discovered', camera: camera({id: 'sim_0', connected: false})})
        expect(app.cameras.value.map((c) => c.id)).toContain('sim_0')
    })

    // Another client's connect: this page's list held no guide camera, so no Guide toggle.
    it('refetches the list when any client connects a camera, once for a burst', async () => {
        await loaded()
        const list = pending<Camera[]>()
        vi.mocked(listCameras)
            .mockReturnValueOnce(list.promise)
            .mockResolvedValue([guide('idle')])

        emitEvent(stream, {type: 'camera_connected', name: 'Neptune-C II'})
        emitEvent(stream, {type: 'camera_disconnected', name: 'Old cam'})
        emitEvent(stream, {type: 'camera_connected', name: 'Neptune-C II'})
        expect(listCameras).toHaveBeenCalledTimes(1)
        list.resolve([])

        await vi.waitFor(() => expect(app.hasGuideCamera.value).toBe(true))
        expect(listCameras).toHaveBeenCalledTimes(2)
    })

    it('reads the settings again when the server says they changed', async () => {
        vi.mocked(getSettings).mockResolvedValue(settings({gain: 300}))
        emitEvent(stream, {type: 'settings_updated'})
        await vi.waitFor(() => expect(app.settings.value?.gain).toBe(300))
    })
})

describe('the settings store', () => {
    let app: AppState

    beforeEach(async () => {
        vi.mocked(getSettings).mockReset()
        vi.mocked(updateSettings).mockReset()
        app = await freshAppState()
    })

    // An edit cost a POST and two or three GETs; the POST's answer is the settings.
    it('adopts a write’s answer without reading the settings again', async () => {
        vi.mocked(updateSettings).mockResolvedValue(settings({gain: 250, use_simulated_camera: true}))

        await expect(app.applySettings({gain: 250})).resolves.toMatchObject({gain: 250})

        expect(updateSettings).toHaveBeenCalledExactlyOnceWith({gain: 250})
        expect(getSettings).not.toHaveBeenCalled()
        expect(app.settings.value?.gain).toBe(250)
        expect(app.simulatorEnabled.value).toBe(true)
    })

    it('leaves the settings alone when a write is refused', async () => {
        vi.mocked(getSettings).mockResolvedValue(settings({gain: 100}))
        await app.refreshSettings()
        vi.mocked(updateSettings).mockRejectedValue(new Error('Refused'))

        await expect(app.applySettings({gain: -1})).rejects.toThrow('Refused')
        expect(app.settings.value?.gain).toBe(100)
    })

    it('shares one read between callers asking while it is in flight, and reads once more after', async () => {
        const first = pending<Settings>()
        const second = pending<Settings>()
        vi.mocked(getSettings).mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise)

        const a = app.refreshSettings()
        const b = app.refreshSettings()
        const c = app.refreshSettings()
        expect(getSettings).toHaveBeenCalledTimes(1)

        first.resolve(settings({gain: 1}))
        await a
        expect(getSettings).toHaveBeenCalledTimes(2)
        second.resolve(settings({gain: 2}))

        await expect(b).resolves.toMatchObject({gain: 2})
        await expect(c).resolves.toMatchObject({gain: 2})
        expect(getSettings).toHaveBeenCalledTimes(2)
    })

    // A read sent beside a write could be answered from before it.
    it('holds a read asked for during a write until the write has landed', async () => {
        const write = pending<Settings>()
        vi.mocked(updateSettings).mockReturnValueOnce(write.promise)
        vi.mocked(getSettings).mockResolvedValue(settings({gain: 250}))

        const applying = app.applySettings({gain: 250})
        const reading = app.refreshSettings()
        await nextTick()
        expect(getSettings).not.toHaveBeenCalled()

        write.resolve(settings({gain: 250}))
        await applying
        await expect(reading).resolves.toMatchObject({gain: 250})
        expect(getSettings).toHaveBeenCalledTimes(1)
    })

    // An older GET resolving last used to overwrite newer settings.
    it('drops an answer older than the one adopted', async () => {
        const read = pending<Settings>()
        vi.mocked(getSettings).mockReturnValueOnce(read.promise)
        vi.mocked(updateSettings).mockResolvedValue(settings({gain: 250}))

        const reading = app.refreshSettings()
        await app.applySettings({gain: 250})
        read.resolve(settings({gain: 0}))
        await reading

        expect(app.settings.value?.gain).toBe(250)
    })

    it('rejects a failed read to its caller, logged once', async () => {
        const error = vi.spyOn(console, 'error').mockImplementation(() => {})
        vi.mocked(getSettings).mockRejectedValue(new Error('offline'))

        await expect(app.refreshSettings()).rejects.toThrow('offline')

        expect(error).toHaveBeenCalledExactlyOnceWith('Failed to load settings:', expect.any(Error))
        error.mockRestore()
    })
})
