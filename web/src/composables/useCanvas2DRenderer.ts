import {ref} from 'vue'
import {rgb8ToRgba8} from '../utils/pixelConversion'
import type {RenderSource, RendererBackend} from './useWebGLRenderer'

/** As `useWebGLRenderer`'s; local, since tests mock that module whole. */
function isImageBitmap(source: RenderSource): source is ImageBitmap {
    return typeof ImageBitmap !== 'undefined' && source instanceof ImageBitmap
}

/**
 * Canvas 2D renderer composable for fallback rendering when WebGL is unavailable
 */
export function useCanvas2DRenderer() {
    const backend = ref<RendererBackend>('unknown')

    let ctx2d: CanvasRenderingContext2D | null = null
    let imageData: ImageData | null = null

    function init(canvas: HTMLCanvasElement | null | undefined) {
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

    /** Whether it drew, as `useWebGLRenderer`'s does, a smaller source stretched over the canvas. */
    function render(
        canvas: HTMLCanvasElement,
        frameData: RenderSource | null,
        width: number,
        height: number,
        sourceWidth = width,
        sourceHeight = height,
    ) {
        if (!ctx2d || !frameData) return false

        if (canvas.width !== width || canvas.height !== height) {
            canvas.width = width
            canvas.height = height
            imageData = null
        }
        const stretched = sourceWidth !== width || sourceHeight !== height

        if (isImageBitmap(frameData)) {
            if (stretched) ctx2d.drawImage(frameData, 0, 0, width, height)
            else ctx2d.drawImage(frameData, 0, 0)
            return true
        }

        if (!imageData || imageData.width !== sourceWidth || imageData.height !== sourceHeight) {
            imageData = ctx2d.createImageData(sourceWidth, sourceHeight)
        }

        rgb8ToRgba8(frameData, imageData.data)
        ctx2d.putImageData(imageData, 0, 0)
        // `putImageData` does not scale; a canvas drawn onto itself is copied first.
        if (stretched) ctx2d.drawImage(canvas, 0, 0, sourceWidth, sourceHeight, 0, 0, width, height)
        return true
    }

    /** No texture to outgrow: the stream's own cap bounds what this draws. */
    function maxSourceEdge() {
        return Infinity
    }

    function cleanup() {
        ctx2d = null
        imageData = null
        backend.value = 'unknown'
    }

    function isInitialized() {
        return backend.value !== 'unknown' && backend.value !== 'none'
    }

    return {
        backend,
        init,
        render,
        maxSourceEdge,
        cleanup,
        isInitialized,
    }
}

export type Canvas2DRenderer = ReturnType<typeof useCanvas2DRenderer>
