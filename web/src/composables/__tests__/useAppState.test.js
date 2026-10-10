import {describe, it, expect, vi, beforeEach} from 'vitest'

vi.mock('../api.js', () => ({
    getSettings: vi.fn(),
    listCameras: vi.fn(),
    getCapabilities: vi.fn(),
}))

import {listCameras} from '../api.js'

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
