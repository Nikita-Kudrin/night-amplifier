import {ref, computed} from 'vue'
import {updateSettings} from '../composables/api.js'
import {COMET_ROI_MIN_PX, STACKING_TYPES} from '../constants'

/**
 * Composable for managing Comet ROI selection and display
 *
 * @param {import('vue').Ref} settings - App settings ref
 * @param {import('vue').Ref} dimensions - Image dimensions ref {width, height}
 * @param {import('vue').Ref} canvasBounds - The canvas's box in container coordinates, kept
 *   current by the view; a `computed` reading `getBoundingClientRect` went stale on zoom or pan
 * @param {import('vue').Ref} containerRef - Container element ref
 * @param {Object} [options]
 * @param {(patch: object) => Promise<object>} [options.save] - Saves the ROI (`useApplySettings`)
 * @returns {Object} ROI selection state and handlers
 */
export function useCometRoi(settings, dimensions, canvasBounds, containerRef, {save = updateSettings} = {}) {
    const isSelectingCometRoi = ref(false)
    const roiSelectionStart = ref(null)
    const roiSelectionEnd = ref(null)

    const isCometMode = computed(() => settings.value?.stacking_type === STACKING_TYPES.COMET)
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
        if (!currentCometRoi.value || !hasFrame.value) return null

        const roi = currentCometRoi.value
        const bounds = canvasBounds.value
        const scaleX = bounds.width / dimensions.value.width
        const scaleY = bounds.height / dimensions.value.height

        return {
            left: roi.x * scaleX + bounds.left,
            top: roi.y * scaleY + bounds.top,
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
        if (!hasFrame.value) return

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

        // Clamp to canvas bounds
        const bounds = canvasBounds.value
        const clampedStartX = Math.max(startX - bounds.left, 0)
        const clampedStartY = Math.max(startY - bounds.top, 0)
        const clampedEndX = Math.min(endX - bounds.left, bounds.width)
        const clampedEndY = Math.min(endY - bounds.top, bounds.height)

        // Convert to image pixel coordinates
        const scaleX = dimensions.value.width / bounds.width
        const scaleY = dimensions.value.height / bounds.height

        const imageRoi = {
            x: Math.round(clampedStartX * scaleX),
            y: Math.round(clampedStartY * scaleY),
            width: Math.round((clampedEndX - clampedStartX) * scaleX),
            height: Math.round((clampedEndY - clampedStartY) * scaleY),
        }

        // Ensure minimum size in image coordinates
        if (imageRoi.width < COMET_ROI_MIN_PX || imageRoi.height < COMET_ROI_MIN_PX) {
            cancelCometRoiSelection()
            return
        }

        // Send to API
        try {
            await save({comet_roi: imageRoi})
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
