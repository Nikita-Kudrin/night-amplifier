import {ref, computed} from 'vue'
import {FIT_MARGIN, ZOOM_LIMITS} from '../constants'
import {useFullscreen} from './useFullscreen'
import type {FullscreenOptions} from './useFullscreen'
import {clamp, fitScale, touchDistance} from '../utils/viewGeometry'

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

/**
 * Pan and zoom interaction composable for canvas-based views.
 *
 * `options` are forwarded to [`useFullscreen`], which owns the fullscreen half —
 * the eyepiece view needs that half without any of the pan/zoom state.
 */
export function usePanZoom(options: FullscreenOptions = {}) {
    const scale = ref(1)
    const position = ref<View['position']>({x: 0, y: 0})
    const isDragging = ref(false)
    const dragStart = ref({x: 0, y: 0})
    const {isFullscreen, toggleFullscreen, handleFullscreenChange} = useFullscreen(options)

    // Touch pinch zoom state
    let initialPinchDistance = 0
    let initialScale = 1

    const canvasStyle = computed(() => ({
        transform: `translate(${position.value.x}px, ${position.value.y}px) scale(${scale.value})`,
        cursor: isDragging.value ? 'grabbing' : 'grab',
    }))

    function handleWheel(e: Pick<WheelEvent, 'deltaY' | 'preventDefault'>) {
        e.preventDefault()
        const delta = e.deltaY > 0 ? ZOOM_LIMITS.wheelZoomOut : ZOOM_LIMITS.wheelZoomIn
        scale.value = clampScale(scale.value * delta)
    }

    function handleMouseDown(e: Pick<MouseEvent, 'button' | 'clientX' | 'clientY'>) {
        if (e.button !== 0) return
        isDragging.value = true
        dragStart.value = {
            x: e.clientX - position.value.x,
            y: e.clientY - position.value.y,
        }
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

    function handleTouchStart(e: TouchGesture) {
        if (e.touches.length === 2) {
            initialPinchDistance = touchDistance(e.touches)
            initialScale = scale.value
        } else if (e.touches.length === 1) {
            isDragging.value = true
            dragStart.value = {
                x: e.touches[0].clientX - position.value.x,
                y: e.touches[0].clientY - position.value.y,
            }
        }
    }

    function handleTouchMove(e: TouchGesture) {
        if (e.touches.length === 2) {
            scale.value = clampScale(initialScale * (touchDistance(e.touches) / initialPinchDistance))
        } else if (e.touches.length === 1 && isDragging.value) {
            position.value = {
                x: e.touches[0].clientX - dragStart.value.x,
                y: e.touches[0].clientY - dragStart.value.y,
            }
        }
    }

    function handleTouchEnd() {
        isDragging.value = false
    }

    function clampScale(value: number) {
        return clamp(value, ZOOM_LIMITS.min, ZOOM_LIMITS.max)
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
        zoomIn,
        zoomOut,
        resetView,
        fitToView,
        toggleFullscreen,
        handleFullscreenChange,
    }
}
