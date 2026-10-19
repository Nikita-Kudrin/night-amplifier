import {ref, computed} from 'vue'
import {FIT_MARGIN, ZOOM_LIMITS} from '../constants'
import {useFullscreen} from './useFullscreen'
import type {FullscreenOptions} from './useFullscreen'
import {clamp, fitScale, touchCenter, touchDistance} from '../utils/viewGeometry'
import type {Point} from '../utils/viewGeometry'

/** What a gesture reads of an event's touches: `TouchList`, or a test's stand-in. */
export type GestureTouches = Parameters<typeof touchDistance>[0]

/** What a pan or pinch reads of a touch event. */
export interface TouchGesture {
    touches: GestureTouches
}

/** A pan and zoom: `transform: translate(position) scale(scale)` about the canvas centre. */
export interface View {
    scale: number
    position: {x: number; y: number}
}

export interface PanZoomOptions extends FullscreenOptions {
    /** The scale the whole image fits at, `null` before a frame; zoom is bounded relative to it. */
    fitScale?: () => number | null
    /**
     * Where, in client coordinates, the untranslated canvas centre sits. With it, a pinch zooms
     * about the point between the fingers and the wheel about the pointer; without, about the
     * canvas centre.
     */
    viewCentre?: () => Point | null
}

/**
 * Pan and zoom interaction composable for canvas-based views.
 *
 * The fullscreen half of `options` is forwarded to [`useFullscreen`], which owns it — the
 * eyepiece view needs that half without any of the pan/zoom state.
 */
export function usePanZoom(options: PanZoomOptions = {}) {
    const scale = ref(1)
    const position = ref<View['position']>({x: 0, y: 0})
    const isDragging = ref(false)
    const dragStart = ref({x: 0, y: 0})
    const {isFullscreen, toggleFullscreen, handleFullscreenChange} = useFullscreen(options)

    // The pinch in progress: where it started, and the view it started from.
    let pinch: {distance: number; centre: Point; view: View} | null = null

    const canvasStyle = computed(() => ({
        transform: `translate(${position.value.x}px, ${position.value.y}px) scale(${scale.value})`,
        cursor: isDragging.value ? 'grabbing' : 'grab',
    }))

    function currentView(): View {
        return {scale: scale.value, position: position.value}
    }

    /**
     * Bounds a zoom from `from`. The bounds widen to take in `from` itself, so a view left
     * outside them (the container grew, a crop reframed it) moves on smoothly instead of
     * jumping to them on the next gesture.
     */
    function clampScale(value: number, from = scale.value) {
        const fit = options.fitScale?.() ?? 1
        const min = Math.min(fit * ZOOM_LIMITS.minOfFit, from)
        const max = Math.max(ZOOM_LIMITS.max, fit, from)
        return clamp(value, min, max)
    }

    /** Zooms `base` to `next`, moving the image point that was under `from` to `to`. */
    function zoomAbout(next: number, from: Point, to: Point, base: View) {
        const centre = options.viewCentre?.()
        scale.value = next
        if (!centre) return
        const k = next / base.scale
        position.value = {
            x: to.x - centre.x - (from.x - centre.x - base.position.x) * k,
            y: to.y - centre.y - (from.y - centre.y - base.position.y) * k,
        }
    }

    function handleWheel(e: Pick<WheelEvent, 'deltaY' | 'clientX' | 'clientY' | 'preventDefault'>) {
        e.preventDefault()
        const delta = e.deltaY > 0 ? ZOOM_LIMITS.wheelZoomOut : ZOOM_LIMITS.wheelZoomIn
        const pointer = {x: e.clientX, y: e.clientY}
        zoomAbout(clampScale(scale.value * delta), pointer, pointer, currentView())
    }

    function startDrag(x: number, y: number) {
        isDragging.value = true
        dragStart.value = {x: x - position.value.x, y: y - position.value.y}
    }

    function handleMouseDown(e: Pick<MouseEvent, 'button' | 'clientX' | 'clientY'>) {
        if (e.button !== 0) return
        startDrag(e.clientX, e.clientY)
    }

    function handleMouseMove(e: Pick<MouseEvent, 'clientX' | 'clientY'>) {
        if (!isDragging.value) return
        position.value = {
            x: e.clientX - dragStart.value.x,
            y: e.clientY - dragStart.value.y,
        }
    }

    function handleMouseUp() {
        isDragging.value = false
    }

    function startPinch(touches: GestureTouches) {
        isDragging.value = false
        pinch = {distance: touchDistance(touches), centre: touchCenter(touches), view: currentView()}
    }

    function handleTouchStart(e: TouchGesture) {
        if (e.touches.length === 2) {
            startPinch(e.touches)
        } else if (e.touches.length === 1) {
            startDrag(e.touches[0].clientX, e.touches[0].clientY)
        }
    }

    function handleTouchMove(e: TouchGesture) {
        if (e.touches.length === 2 && pinch) {
            const next = clampScale(pinch.view.scale * (touchDistance(e.touches) / pinch.distance), pinch.view.scale)
            zoomAbout(next, pinch.centre, touchCenter(e.touches), pinch.view)
        } else if (e.touches.length === 1 && isDragging.value) {
            position.value = {
                x: e.touches[0].clientX - dragStart.value.x,
                y: e.touches[0].clientY - dragStart.value.y,
            }
        }
    }

    /** `e.touches` are the fingers still down: a pinch losing one carries on as a pan from here. */
    function handleTouchEnd(e: TouchGesture) {
        pinch = null
        isDragging.value = false
        if (e.touches.length === 2) {
            startPinch(e.touches)
        } else if (e.touches.length === 1) {
            startDrag(e.touches[0].clientX, e.touches[0].clientY)
        }
    }

    function handleTouchCancel() {
        pinch = null
        isDragging.value = false
    }

    function zoomIn() {
        scale.value = clampScale(scale.value * ZOOM_LIMITS.zoomInFactor)
    }

    function zoomOut() {
        scale.value = clampScale(scale.value * ZOOM_LIMITS.zoomOutFactor)
    }

    function resetView() {
        scale.value = 1
        position.value = {x: 0, y: 0}
    }

    function fitToView(
        containerRect: Pick<DOMRect, 'width' | 'height'> | null | undefined,
        canvasWidth: number | null | undefined,
        canvasHeight: number | null | undefined,
    ) {
        if (!containerRect || !canvasWidth || !canvasHeight) return

        scale.value = fitScale(containerRect, canvasWidth, canvasHeight, FIT_MARGIN)
        position.value = {x: 0, y: 0}
    }

    return {
        scale,
        position,
        isDragging,
        isFullscreen,
        canvasStyle,
        handleWheel,
        handleMouseDown,
        handleMouseMove,
        handleMouseUp,
        handleTouchStart,
        handleTouchMove,
        handleTouchEnd,
        handleTouchCancel,
        zoomIn,
        zoomOut,
        resetView,
        fitToView,
        toggleFullscreen,
        handleFullscreenChange,
    }
}
