import {ref, computed} from 'vue'
import {ZOOM_LIMITS} from '../constants'

/**
 * Pan and zoom interaction composable for canvas-based views
 */
export function usePanZoom() {
    const scale = ref(1)
    const position = ref({x: 0, y: 0})
    const isDragging = ref(false)
    const dragStart = ref({x: 0, y: 0})
    const isFullscreen = ref(false)

    // Touch pinch zoom state
    let initialPinchDistance = 0
    let initialScale = 1

    const canvasStyle = computed(() => ({
        transform: `translate(${position.value.x}px, ${position.value.y}px) scale(${scale.value})`,
        cursor: isDragging.value ? 'grabbing' : 'grab',
    }))

    function handleWheel(e) {
        e.preventDefault()
        const delta = e.deltaY > 0 ? ZOOM_LIMITS.wheelZoomOut : ZOOM_LIMITS.wheelZoomIn
        scale.value = clampScale(scale.value * delta)
    }

    function handleMouseDown(e) {
        if (e.button !== 0) return
        isDragging.value = true
        dragStart.value = {
            x: e.clientX - position.value.x,
            y: e.clientY - position.value.y,
        }
    }

    function handleMouseMove(e) {
        if (!isDragging.value) return
        position.value = {
            x: e.clientX - dragStart.value.x,
            y: e.clientY - dragStart.value.y,
        }
    }

    function handleMouseUp() {
        isDragging.value = false
    }

    function handleTouchStart(e) {
        if (e.touches.length === 2) {
            const dx = e.touches[0].clientX - e.touches[1].clientX
            const dy = e.touches[0].clientY - e.touches[1].clientY
            initialPinchDistance = Math.hypot(dx, dy)
            initialScale = scale.value
        } else if (e.touches.length === 1) {
            isDragging.value = true
            dragStart.value = {
                x: e.touches[0].clientX - position.value.x,
                y: e.touches[0].clientY - position.value.y,
            }
        }
    }

    function handleTouchMove(e) {
        if (e.touches.length === 2) {
            const dx = e.touches[0].clientX - e.touches[1].clientX
            const dy = e.touches[0].clientY - e.touches[1].clientY
            const distance = Math.hypot(dx, dy)
            scale.value = clampScale(initialScale * (distance / initialPinchDistance))
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

    function clampScale(value) {
        return Math.max(ZOOM_LIMITS.min, Math.min(ZOOM_LIMITS.max, value))
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

    function fitToView(containerRect, canvasWidth, canvasHeight) {
        if (!containerRect || !canvasWidth || !canvasHeight) return

        const scaleX = containerRect.width / canvasWidth
        const scaleY = containerRect.height / canvasHeight
        scale.value = Math.min(scaleX, scaleY, 1) * 0.95
        position.value = {x: 0, y: 0}
    }

    function toggleFullscreen(containerElement) {
        if (!document.fullscreenElement) {
            containerElement?.requestFullscreen()
            isFullscreen.value = true
        } else {
            document.exitFullscreen()
            isFullscreen.value = false
        }
    }

    function handleFullscreenChange() {
        isFullscreen.value = !!document.fullscreenElement
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
