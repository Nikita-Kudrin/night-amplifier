/**
 * How long a requested fit keeps following the container. Each reshape restarts it,
 * so a URL bar sliding back in after the first resize is still followed.
 */
export const FIT_SETTLE_MS = 500

/**
 * Fits the view once the layout has settled after the viewport changed shape.
 * Leaving fullscreen and rotating land before the browser re-lays the container,
 * so fitting on the event risks measuring the size being left. `requestFit` arms
 * instead: any real size change while armed fits, or one fit runs when the window
 * closes. Unarmed reshapes are ignored (sidebar drags keep the user's framing);
 * listeners stay the caller's to register, as with `useFullscreen`.
 */
export function useAutoFit({fit, settleMs = FIT_SETTLE_MS}: {fit: () => void; settleMs?: number}) {
    let settleTimer: ReturnType<typeof setTimeout> | null = null
    let fitted = false
    let lastSize: {width: number; height: number} | null = null

    function requestFit() {
        if (!settleTimer) fitted = false
        restartSettleTimer()
    }

    function handleContainerResize(width: number, height: number) {
        const reshaped = lastSize !== null && (width !== lastSize.width || height !== lastSize.height)
        lastSize = {width, height}
        if (!reshaped || !settleTimer) return
        fitted = true
        restartSettleTimer()
        fit()
    }

    function restartSettleTimer() {
        clearTimeout(settleTimer ?? undefined)
        settleTimer = setTimeout(settle, settleMs)
    }

    function settle() {
        settleTimer = null
        if (!fitted) fit()
    }

    function dispose() {
        clearTimeout(settleTimer ?? undefined)
        settleTimer = null
    }

    return {requestFit, handleContainerResize, dispose}
}
