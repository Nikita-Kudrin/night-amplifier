import {ref, computed} from 'vue'
import {updateSettings} from '../composables/api.js'

/**
 * Composable for managing Comet ROI selection and display
 *
 * @param {import('vue').Ref} settings - App settings ref
 * @param {import('vue').Ref} dimensions - Image dimensions ref {width, height}
 * @param {import('vue').Ref} canvasRef - Target canvas element ref
 * @param {import('vue').Ref} containerRef - Container element ref
 * @returns {Object} ROI selection state and handlers
 */
export function useCometRoi(settings, dimensions, canvasRef, containerRef) {
    const isSelectingCometRoi = ref(false)
    const roiSelectionStart = ref(null)
    const roiSelectionEnd = ref(null)

    const isCometMode = computed(() => settings.value?.stacking_type === 'comet')
    const currentCometRoi = computed(() => settings.value?.comet_roi)
    const hasFrame = computed(() => dimensions.value.width > 0)

    // Selection rectangle while drawing (screen coordinates)
    const selectionRect = computed(() => {
        if (!roiSelectionStart.value || !roiSelectionEnd.value) return null

        const startX = Math.min(roiSelectionStart.value.x, roiSelectionEnd.value.x)
        const startY = Math.min(roiSelectionStart.value.y, roiSelectionEnd.value.y)
        const endX = Math.max(roiSelectionStart.value.x, roiSelectionEnd.value.x)
        const endY = Math.max(roiSelectionStart.value.y, roiSelectionEnd.value.y)

        return {
            left: startX,
            top: startY,
            width: endX - startX,
            height: endY - startY,
        }
    })

    // Computed ROI rectangle in screen coordinates for display
    const roiDisplayRect = computed(() => {
        if (!currentCometRoi.value || !canvasRef.value || !hasFrame.value) return null

        const roi = currentCometRoi.value
        const canvas = canvasRef.value
        const canvasRect = canvas.getBoundingClientRect()
        const containerRect = containerRef.value?.getBoundingClientRect() || {left: 0, top: 0}

        // Convert image coordinates to screen coordinates
        const scaleX = canvasRect.width / dimensions.value.width
        const scaleY = canvasRect.height / dimensions.value.height

        const canvasLeft = canvasRect.left - containerRect.left
        const canvasTop = canvasRect.top - containerRect.top

        return {
            left: roi.x * scaleX + canvasLeft,
            top: roi.y * scaleY + canvasTop,
            width: roi.width * scaleX,
            height: roi.height * scaleY,
        }
    })

    function startCometRoiSelection() {
        isSelectingCometRoi.value = true
        roiSelectionStart.value = null
        roiSelectionEnd.value = null
    }

    function cancelCometRoiSelection() {
        isSelectingCometRoi.value = false
        roiSelectionStart.value = null
        roiSelectionEnd.value = null
    }

    function handleMouseDown(event) {
        if (!isSelectingCometRoi.value || !containerRef.value) return

        const rect = containerRef.value.getBoundingClientRect()
        roiSelectionStart.value = {
            x: event.clientX - rect.left,
            y: event.clientY - rect.top,
        }
        roiSelectionEnd.value = roiSelectionStart.value
    }

    function handleMouseMove(event) {
        if (!isSelectingCometRoi.value || !roiSelectionStart.value || !containerRef.value) return

        const rect = containerRef.value.getBoundingClientRect()
        roiSelectionEnd.value = {
            x: event.clientX - rect.left,
            y: event.clientY - rect.top,
        }
    }

    async function handleMouseUp() {
        if (!isSelectingCometRoi.value || !roiSelectionStart.value || !roiSelectionEnd.value) return
        if (!canvasRef.value || !hasFrame.value) return

        const canvas = canvasRef.value
        const canvasRect = canvas.getBoundingClientRect()
        const containerRect = containerRef.value.getBoundingClientRect()

        // Get selection bounds relative to container
        const startX = Math.min(roiSelectionStart.value.x, roiSelectionEnd.value.x)
        const startY = Math.min(roiSelectionStart.value.y, roiSelectionEnd.value.y)
        const endX = Math.max(roiSelectionStart.value.x, roiSelectionEnd.value.x)
        const endY = Math.max(roiSelectionStart.value.y, roiSelectionEnd.value.y)

        // Minimum size check (screen pixels)
        if (endX - startX < 10 || endY - startY < 10) {
            cancelCometRoiSelection()
            return
        }

        // Convert screen coordinates to image coordinates
        const canvasLeft = canvasRect.left - containerRect.left
        const canvasTop = canvasRect.top - containerRect.top

        // Clamp to canvas bounds
        const clampedStartX = Math.max(startX - canvasLeft, 0)
        const clampedStartY = Math.max(startY - canvasTop, 0)
        const clampedEndX = Math.min(endX - canvasLeft, canvasRect.width)
        const clampedEndY = Math.min(endY - canvasTop, canvasRect.height)

        // Convert to image pixel coordinates
        const scaleX = dimensions.value.width / canvasRect.width
        const scaleY = dimensions.value.height / canvasRect.height

        const imageRoi = {
            x: Math.round(clampedStartX * scaleX),
            y: Math.round(clampedStartY * scaleY),
            width: Math.round((clampedEndX - clampedStartX) * scaleX),
            height: Math.round((clampedEndY - clampedStartY) * scaleY),
        }

        // Ensure minimum size in image coordinates
        if (imageRoi.width < 20 || imageRoi.height < 20) {
            cancelCometRoiSelection()
            return
        }

        // Send to API
        try {
            await updateSettings({comet_roi: imageRoi})
            console.log('[useCometRoi] Comet ROI set:', imageRoi)
        } catch (e) {
            console.error('[useCometRoi] Failed to set comet ROI:', e)
        }

        cancelCometRoiSelection()
    }

    return {
        isSelectingCometRoi,
        isCometMode,
        selectionRect,
        roiDisplayRect,
        startCometRoiSelection,
        cancelCometRoiSelection,
        handleMouseDown,
        handleMouseMove,
        handleMouseUp,
    }
}
