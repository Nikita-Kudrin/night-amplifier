import {ref} from 'vue'
import {rgb8ToRgba8} from '../utils/pixelConversion.js'

/**
 * Canvas 2D renderer composable for fallback rendering when WebGL is unavailable
 */
export function useCanvas2DRenderer() {
    const backend = ref('unknown')

    let ctx2d = null
    let imageData = null

    function init(canvas) {
        if (!canvas) {
            console.warn('[Canvas2DRenderer] Canvas element not available')
            return false
        }

        console.log('[Canvas2DRenderer] Initializing Canvas 2D (fallback)...')

        ctx2d = canvas.getContext('2d')
        if (!ctx2d) {
            console.error('[Canvas2DRenderer] Canvas 2D not available')
            return false
        }

        backend.value = 'canvas2d'
        console.log('[Canvas2DRenderer] Canvas 2D initialization complete - software rendering (8-bit)')
        console.warn('[Canvas2DRenderer] Performance may be reduced without WebGL acceleration')
        return true
    }

    function render(canvas, frameData, width, height) {
        if (!ctx2d || !frameData) return

        if (canvas.width !== width || canvas.height !== height) {
            canvas.width = width
            canvas.height = height
            imageData = null
        }

        if (!imageData || imageData.width !== width || imageData.height !== height) {
            imageData = ctx2d.createImageData(width, height)
        }

        const rgba8 = rgb8ToRgba8(frameData, width, height)
        imageData.data.set(rgba8)

        ctx2d.putImageData(imageData, 0, 0)
    }

    function cleanup() {
        ctx2d = null
        imageData = null
        backend.value = 'unknown'
    }

    function isInitialized() {
        return ctx2d !== null
    }

    return {
        backend,
        init,
        render,
        cleanup,
        isInitialized,
    }
}
