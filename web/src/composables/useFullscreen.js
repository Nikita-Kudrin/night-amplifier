import {ref} from 'vue'

/**
 * Fullscreen state for one element.
 *
 * `isFullscreen` follows the `fullscreenchange` event rather than the click that
 * asked for it: `requestFullscreen()` can be refused, and the browser leaves
 * fullscreen on its own (Esc, a gesture, another element taking over), so the
 * request is not the state. It is also what makes the transitions observable —
 * `onChange` fires on the false -> true and true -> false edges only.
 *
 * `onChange` runs after the browser confirms, which is the point: the viewport
 * is still the old size until then, so a fit-all driven from the click would fit
 * to the window it just left. Leaving fullscreen shrinks the viewport the same
 * way, so it needs the same fit.
 *
 * The `fullscreenchange` listener stays the caller's to register — the two views
 * using this already own a `document` listener each, and a composable-owned hook
 * would not survive `usePanZoom()` being called outside a component in its tests.
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
