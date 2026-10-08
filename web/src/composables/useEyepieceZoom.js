import {ref, computed} from 'vue'

/** The eyepiece zooms up to 2x and never out past the screen. */
const MAX_SCALE = 2
const WHEEL_SENSITIVITY = 0.001

/**
 * Wheel and pinch zoom for the eyepiece view, panned with one finger while zoomed.
 *
 * Unlike `usePanZoom` (the live view), the view fills the window: the pan is clamped so
 * no edge of the image moves inside the screen, and 1x is the floor. Gestures do nothing
 * while `enabled()` is false.
 *
 * @param {() => boolean} enabled
 */
export function useEyepieceZoom(enabled) {
    const scale = ref(1)
    const panX = ref(0)
    const panY = ref(0)
    const isPinching = ref(false)

    // Where the current gesture started.
    let initialDist = 0
    let initialScale = 1
    let initialCx = 0
    let initialCy = 0
    let initialPanX = 0
    let initialPanY = 0

    const style = computed(() => {
        if (scale.value === 1) return {}
        return {
            transform: `translate(${panX.value}px, ${panY.value}px) scale(${scale.value})`,
            transformOrigin: '0 0',
        }
    })

    function clampPan(x, y, s) {
        if (s <= 1) return {x: 0, y: 0}
        const minX = window.innerWidth * (1 - s)
        const minY = window.innerHeight * (1 - s)
        return {
            x: Math.min(0, Math.max(minX, x)),
            y: Math.min(0, Math.max(minY, y)),
        }
    }

    function setView(x, y, s) {
        const clamped = clampPan(x, y, s)
        panX.value = clamped.x
        panY.value = clamped.y
        scale.value = s
    }

    function reset() {
        scale.value = 1
        panX.value = 0
        panY.value = 0
    }

    function getDist(touches) {
        const dx = touches[0].clientX - touches[1].clientX
        const dy = touches[0].clientY - touches[1].clientY
        return Math.sqrt(dx * dx + dy * dy)
    }

    function getCenter(touches) {
        if (touches.length === 1) {
            return {x: touches[0].clientX, y: touches[0].clientY}
        }
        return {
            x: (touches[0].clientX + touches[1].clientX) / 2,
            y: (touches[0].clientY + touches[1].clientY) / 2,
        }
    }

    function anchorGesture(touches) {
        const center = getCenter(touches)
        initialCx = center.x
        initialCy = center.y
        initialPanX = panX.value
        initialPanY = panY.value
    }

    /** Zoom about the pointer. */
    function wheel(e) {
        if (!enabled()) return
        const oldScale = scale.value
        const newScale = Math.min(MAX_SCALE, Math.max(1, oldScale - e.deltaY * WHEEL_SENSITIVITY))
        if (newScale === oldScale) return

        const cx = e.clientX
        const cy = e.clientY
        setView(
            cx - (cx - panX.value) * (newScale / oldScale),
            cy - (cy - panY.value) * (newScale / oldScale),
            newScale
        )
    }

    function touchStart(e) {
        if (!enabled()) return
        if (e.touches.length === 2) {
            isPinching.value = true
            initialDist = getDist(e.touches)
            initialScale = scale.value
            anchorGesture(e.touches)
        } else if (e.touches.length === 1 && scale.value > 1) {
            anchorGesture(e.touches)
        }
    }

    function touchMove(e) {
        if (!enabled()) return
        if (e.touches.length === 2 && isPinching.value) {
            const newScale = Math.min(MAX_SCALE, Math.max(1, initialScale * (getDist(e.touches) / initialDist)))
            const center = getCenter(e.touches)
            setView(
                center.x - (initialCx - initialPanX) * (newScale / initialScale),
                center.y - (initialCy - initialPanY) * (newScale / initialScale),
                newScale
            )
        } else if (e.touches.length === 1 && scale.value > 1 && !isPinching.value) {
            const center = getCenter(e.touches)
            setView(initialPanX + center.x - initialCx, initialPanY + center.y - initialCy, scale.value)
        }
    }

    /** A pinch that drops to one finger carries on as a pan from where it is now. */
    function touchEnd(e) {
        if (e.touches.length < 2) {
            isPinching.value = false
        }
        if (e.touches.length === 1 && scale.value > 1) {
            anchorGesture(e.touches)
        }
    }

    function touchCancel() {
        isPinching.value = false
    }

    return {scale, style, reset, wheel, touchStart, touchMove, touchEnd, touchCancel}
}
