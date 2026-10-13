import {ref} from 'vue'

export interface FullscreenOptions {
    /** Called on each edge the browser confirms: `true` on entering, `false` on leaving. */
    onChange?: (entered: boolean) => void
}

/**
 * Fullscreen state for one element. `isFullscreen` tracks `fullscreenchange`, not the click:
 * `requestFullscreen()` can be refused, and the browser can exit on its own (Esc, gesture, takeover).
 * `onChange` fires only on those resulting edges, after the browser confirms — so callers
 * that fit to the viewport must wait for it, since the size is still the old one until then.
 * The listener stays the caller's to register: both views here already own a `document` listener,
 * and a composable-owned hook wouldn't survive `usePanZoom()` running outside a component in tests.
 */
export function useFullscreen({onChange}: FullscreenOptions = {}) {
    const isFullscreen = ref(false)

    function toggleFullscreen(element: Element | null | undefined) {
        // Either can be refused (no user gesture, a frame without the permission); the
        // state follows `fullscreenchange`, so a refusal only needs saying.
        const request = document.fullscreenElement ? document.exitFullscreen() : element?.requestFullscreen()
        request?.catch?.((e: {message?: unknown} | null | undefined) => console.warn('Fullscreen refused:', e?.message ?? e))
    }

    function handleFullscreenChange() {
        const entered = !!document.fullscreenElement
        const wasFullscreen = isFullscreen.value
        isFullscreen.value = entered
        if (entered !== wasFullscreen) onChange?.(entered)
    }

    return {isFullscreen, toggleFullscreen, handleFullscreenChange}
}
