import {ref} from 'vue'

/**
 * Fullscreen state for one element. `isFullscreen` tracks `fullscreenchange`, not the click:
 * `requestFullscreen()` can be refused, and the browser can exit on its own (Esc, gesture, takeover).
 * `onChange` fires only on those resulting edges, after the browser confirms — so callers
 * that fit to the viewport must wait for it, since the size is still the old one until then.
 * The listener stays the caller's to register: both views here already own a `document` listener,
 * and a composable-owned hook wouldn't survive `usePanZoom()` running outside a component in tests.
 */
export function useFullscreen({onChange} = {}) {
    const isFullscreen = ref(false)

    function toggleFullscreen(element) {
        if (!document.fullscreenElement) {
            element?.requestFullscreen()
            return
        }
        document.exitFullscreen()
    }

    function handleFullscreenChange() {
        const entered = !!document.fullscreenElement
        const wasFullscreen = isFullscreen.value
        isFullscreen.value = entered
        if (entered !== wasFullscreen) onChange?.(entered)
    }

    return {isFullscreen, toggleFullscreen, handleFullscreenChange}
}
