import {effectScope, nextTick, ref} from 'vue'
import type {EffectScope} from 'vue'
import {
    RENDERER_RELOAD_LIMIT,
    RENDERER_RELOAD_WINDOW_MS,
    RENDERER_RELOADS_KEY,
    useRendererRecovery,
} from './useRendererRecovery'

describe('useRendererRecovery', () => {
    const reload = vi.fn()
    let scope: EffectScope

    beforeEach(() => {
        vi.stubGlobal('location', {...window.location, reload})
        vi.spyOn(console, 'warn').mockImplementation(() => {})
        sessionStorage.clear()
        scope = effectScope()
    })

    afterEach(() => {
        scope.stop()
        setVisibility('visible')
        sessionStorage.clear()
        vi.unstubAllGlobals()
        vi.restoreAllMocks()
        reload.mockClear()
    })

    function setVisibility(state: DocumentVisibilityState) {
        Object.defineProperty(document, 'visibilityState', {value: state, configurable: true})
        document.dispatchEvent(new Event('visibilitychange'))
    }

    function recover(connected = ref(true)) {
        const recovery = scope.run(() => useRendererRecovery(connected))
        if (!recovery) throw new Error('scope stopped')
        return {...recovery, connected}
    }

    /** A page load after `reloads` earlier ones, each `apart` ms before the next. */
    function afterReloads(reloads: number, apart = 1000) {
        const now = Date.now()
        sessionStorage.setItem(RENDERER_RELOADS_KEY, JSON.stringify(Array.from({length: reloads}, (_, i) => now - (reloads - i) * apart)))
    }

    it('reloads once the renderer is lost, and counts the reload', async () => {
        const {onRendererLost, webgl} = recover()
        expect(webgl).toBe(true)

        onRendererLost()
        await nextTick()

        expect(reload).toHaveBeenCalledTimes(1)
        expect(JSON.parse(sessionStorage.getItem(RENDERER_RELOADS_KEY) ?? '[]')).toHaveLength(1)
    })

    // Chromium never retries a page that failed to load.
    it('waits for the server', async () => {
        const {onRendererLost, connected} = recover(ref(false))

        onRendererLost()
        await nextTick()
        expect(reload).not.toHaveBeenCalled()

        connected.value = true
        await nextTick()
        expect(reload).toHaveBeenCalledTimes(1)
    })

    // A phone's hidden tab would come back reloaded, out of fullscreen and zoom.
    it('waits until the page is shown', async () => {
        setVisibility('hidden')
        const {onRendererLost} = recover()

        onRendererLost()
        await nextTick()
        expect(reload).not.toHaveBeenCalled()

        setVisibility('visible')
        await nextTick()
        expect(reload).toHaveBeenCalledTimes(1)
    })

    it('does nothing while the renderer is fine', async () => {
        const {connected} = recover(ref(false))
        connected.value = true
        setVisibility('hidden')
        setVisibility('visible')
        await nextTick()

        expect(reload).not.toHaveBeenCalled()
    })

    // A GPU that crashes on every upload would otherwise reload the page all night.
    it(`draws without WebGL after ${RENDERER_RELOAD_LIMIT} recent reloads`, () => {
        afterReloads(RENDERER_RELOAD_LIMIT - 1)
        expect(recover().webgl).toBe(true)

        afterReloads(RENDERER_RELOAD_LIMIT)
        expect(recover().webgl).toBe(false)
    })

    it('forgets reloads older than the window', async () => {
        afterReloads(RENDERER_RELOAD_LIMIT, RENDERER_RELOAD_WINDOW_MS)
        const {webgl, onRendererLost} = recover()
        expect(webgl).toBe(true)

        onRendererLost()
        await nextTick()
        // Only this one is kept: the old ones are outside the window.
        expect(JSON.parse(sessionStorage.getItem(RENDERER_RELOADS_KEY) ?? '[]')).toHaveLength(1)
    })

    it('tries WebGL, and still reloads, without storage or with garbage in it', async () => {
        sessionStorage.setItem(RENDERER_RELOADS_KEY, '{not json')
        expect(recover().webgl).toBe(true)
        sessionStorage.setItem(RENDERER_RELOADS_KEY, '{"a": 1}')
        expect(recover().webgl).toBe(true)

        vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => { throw new Error('denied') })
        vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('denied') })
        const {webgl, onRendererLost} = recover()
        onRendererLost()
        await nextTick()

        expect(webgl).toBe(true)
        expect(reload).toHaveBeenCalledTimes(1)
    })
})
