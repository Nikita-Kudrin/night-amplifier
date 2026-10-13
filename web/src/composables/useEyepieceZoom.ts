import {ref, computed} from 'vue'
import {EYEPIECE_ZOOM} from '../constants'
import {clamp, touchCenter, touchDistance} from '../utils/viewGeometry.js'
import type {GestureTouches, TouchGesture} from './usePanZoom.js'

const zoomed = (value: number) => clamp(value, 1, EYEPIECE_ZOOM.max)

/**
 * Wheel and pinch zoom for the eyepiece view, panned with one finger while zoomed.
 *
 * Unlike `usePanZoom` (the live view), the view fills the window: the pan is clamped so
 * no edge of the image moves inside the screen, and 1x is the floor. Gestures do nothing
 * while `enabled()` is false.
 */
export function useEyepieceZoom(enabled: () => boolean) {
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

    const style = computed((): {transform?: string; transformOrigin?: string} => {
        if (scale.value === 1) return {}
        return {
            transform: `translate(${panX.value}px, ${panY.value}px) scale(${scale.value})`,
            transformOrigin: '0 0',
        }
    })

    function clampPan(x: number, y: number, s: number) {
        if (s <= 1) return {x: 0, y: 0}
        const minX = window.innerWidth * (1 - s)
        const minY = window.innerHeight * (1 - s)
        return {x: clamp(x, minX, 0), y: clamp(y, minY, 0)}
    }

    function setView(x: number, y: number, s: number) {
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

    function anchorGesture(touches: GestureTouches) {
        const center = touchCenter(touches)
        initialCx = center.x
        initialCy = center.y
        initialPanX = panX.value
        initialPanY = panY.value
    }

    /** Zoom about the pointer. */
    function wheel(e: Pick<WheelEvent, 'deltaY' | 'clientX' | 'clientY'>) {
        if (!enabled()) return
        const oldScale = scale.value
        const newScale = zoomed(oldScale - e.deltaY * EYEPIECE_ZOOM.wheelSensitivity)
        if (newScale === oldScale) return

        const cx = e.clientX
        const cy = e.clientY
        setView(
            cx - (cx - panX.value) * (newScale / oldScale),
            cy - (cy - panY.value) * (newScale / oldScale),
            newScale
        )
    }

    function touchStart(e: TouchGesture) {
        if (!enabled()) return
        if (e.touches.length === 2) {
            isPinching.value = true
            initialDist = touchDistance(e.touches)
            initialScale = scale.value
            anchorGesture(e.touches)
        } else if (e.touches.length === 1 && scale.value > 1) {
            anchorGesture(e.touches)
        }
    }

    function touchMove(e: TouchGesture) {
        if (!enabled()) return
        if (e.touches.length === 2 && isPinching.value) {
            const newScale = zoomed(initialScale * (touchDistance(e.touches) / initialDist))
            const center = touchCenter(e.touches)
            setView(
                center.x - (initialCx - initialPanX) * (newScale / initialScale),
                center.y - (initialCy - initialPanY) * (newScale / initialScale),
                newScale
            )
        } else if (e.touches.length === 1 && scale.value > 1 && !isPinching.value) {
            const center = touchCenter(e.touches)
            setView(initialPanX + center.x - initialCx, initialPanY + center.y - initialCy, scale.value)
        }
    }

    /** A pinch that drops to one finger carries on as a pan from where it is now. */
    function touchEnd(e: TouchGesture) {
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
