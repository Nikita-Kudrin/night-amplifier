import {ref, watch} from 'vue'
import type {Ref} from 'vue'
import {useEventListener} from './useEventListener'

/** Reloads for a lost renderer within `RENDERER_RELOAD_WINDOW_MS` after which the next load skips WebGL. */
export const RENDERER_RELOAD_LIMIT = 2
export const RENDERER_RELOAD_WINDOW_MS = 5 * 60 * 1000
/** Per tab, so a relaunched browser tries WebGL again. */
export const RENDERER_RELOADS_KEY = 'na.rendererReloads'

function recentReloads(now: number): number[] {
    try {
        const stored: unknown = JSON.parse(sessionStorage.getItem(RENDERER_RELOADS_KEY) ?? '[]')
        if (!Array.isArray(stored)) return []
        return stored.filter((at): at is number => typeof at === 'number' && now - at < RENDERER_RELOAD_WINDOW_MS)
    } catch {
        return []
    }
}

function recordReload(now: number) {
    try {
        sessionStorage.setItem(RENDERER_RELOADS_KEY, JSON.stringify([...recentReloads(now), now]))
    } catch {
        // No storage (a private window): reloads go uncounted, and every load tries WebGL.
    }
}

/**
 * Reloads the page when its renderer is lost for good (`useFrameRenderer`'s `onRendererLost`):
 * a display nobody can reload by hand stays black otherwise. Only while the server answers
 * (Chromium never retries a page that failed to load) and the page is shown (a phone's
 * hidden tab would come back reloaded, out of fullscreen). A GPU that crashes on every
 * upload would reload the page all night: after `RENDERER_RELOAD_LIMIT` reloads in
 * `RENDERER_RELOAD_WINDOW_MS`, `webgl` is `false` and the view draws with Canvas 2D.
 */
export function useRendererRecovery(connected: Readonly<Ref<boolean>>) {
    const webgl = recentReloads(Date.now()).length < RENDERER_RELOAD_LIMIT
    if (!webgl) console.warn('[RendererRecovery] WebGL kept being lost; drawing with Canvas 2D')

    const lost = ref(false)
    const shown = ref(document.visibilityState !== 'hidden')
    useEventListener(document, 'visibilitychange', () => {
        shown.value = document.visibilityState !== 'hidden'
    })

    watch([lost, connected, shown], ([isLost, up, isShown]) => {
        if (!isLost || !up || !isShown) return
        recordReload(Date.now())
        window.location.reload()
    })

    return {webgl, onRendererLost: () => { lost.value = true }}
}
