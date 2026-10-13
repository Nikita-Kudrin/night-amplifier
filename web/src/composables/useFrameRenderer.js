import {computed, ref, watch} from 'vue'
import {tryOnBeforeUnmount, tryOnMounted} from './lifecycle.js'
import {useWebGLRenderer} from './useWebGLRenderer.js'
import {useCanvas2DRenderer} from './useCanvas2DRenderer.js'
import {formatResolution} from '../utils/format.js'

/** What the debug readout calls each backend; `jpeg` is the stream's, not a renderer's. */
export const BACKEND_LABELS = Object.freeze({
    jpeg: 'Dynamic JPEG',
    'webgl2-8bit': 'WebGL2 8-bit',
    webgl1: 'WebGL1',
    canvas2d: 'Canvas 2D',
    none: 'No renderer',
    unknown: '...',
})

/** `backend`'s label, with the frame size once there is one. */
export function backendLabel(backend, {width, height} = {}) {
    const label = BACKEND_LABELS[backend] ?? backend
    if (!(width > 0) || backend === 'none' || backend === 'unknown') return label
    return `${label} (${formatResolution(width, height)})`
}

/**
 * One canvas's renderer: WebGL2, then WebGL1, then Canvas 2D. `useWebGLRenderer` probes
 * each GL version on a scratch canvas first, so a failed one leaves this canvas free
 * for the next.
 */
function createSurface(canvas, onRestored) {
    const webgl = useWebGLRenderer({onRestored})
    const renderer = webgl.init(canvas) ? webgl : useCanvas2DRenderer()
    if (renderer !== webgl && !renderer.init(canvas)) {
        return {backend: () => 'none', draw() {}, cleanup() {}}
    }
    return {
        backend: () => renderer.backend.value,
        draw: (image) => renderer.render(canvas, image.source, image.width, image.height),
        cleanup: renderer.cleanup,
    }
}

/**
 * Draws an image stream's frames onto the canvases `canvases()` names: one for the live
 * view, two for a binoview eyepiece — a frame is decoded once and drawn on each.
 *
 * A JPEG decodes asynchronously, so decodes can finish out of order: only the newest
 * frame's bitmap is drawn, any other is closed. The last image is held, so a restored
 * GL context or a newly shown canvas is drawn without waiting for the next frame (an idle
 * stream sends none); a null frame (`no_frame`, a cleared view) drops it.
 *
 * @param {Object} options
 * @param {import('vue').Ref} options.frameData - The stream's frame: RGB8 bytes or a JPEG blob
 * @param {import('vue').Ref<{width: number, height: number}>} options.dimensions
 * @param {import('vue').Ref<boolean>} options.isJpeg
 * @param {() => (HTMLCanvasElement|null)[]} options.canvases - The canvases on screen now
 * @param {(size: {width: number, height: number}) => void} [options.onSizeChange] - A frame
 *   of a new size was drawn: the canvas was resized, so whatever sits over it re-measures.
 */
export function useFrameRenderer({frameData, dimensions, isJpeg, canvases, onSizeChange}) {
    const surfaces = new Map()
    const rendererBackend = ref('unknown')
    let image = null
    let generation = 0
    let drawnSize = null

    function surfaceFor(canvas) {
        let surface = surfaces.get(canvas)
        if (!surface) {
            surface = createSurface(canvas, draw)
            surfaces.set(canvas, surface)
        }
        return surface
    }

    /** Readies every canvas on screen, and frees the renderers of those that are gone. */
    function attach() {
        const targets = canvases().filter(Boolean)
        for (const [canvas, surface] of surfaces) {
            if (targets.includes(canvas)) continue
            surface.cleanup()
            surfaces.delete(canvas)
        }
        const ready = targets.map(surfaceFor)
        rendererBackend.value = ready.length ? ready[0].backend() : 'none'
        return ready
    }

    /** Draws the held image again: after a canvas appeared, or its context came back. */
    function draw() {
        const ready = attach()
        if (!image) return
        ready.forEach((surface) => surface.draw(image))
        const {width, height} = image
        if (drawnSize?.width === width && drawnSize?.height === height) return
        drawnSize = {width, height}
        onSizeChange?.(drawnSize)
    }

    function hold(next) {
        image?.close?.()
        image = next
    }

    async function show(frame) {
        const current = ++generation
        if (!frame) {
            hold(null)
            return
        }
        if (!isJpeg.value) {
            hold({source: frame, ...dimensions.value})
            draw()
            return
        }
        let bitmap
        try {
            bitmap = await window.createImageBitmap(frame)
        } catch {
            return // not decodable; the next frame replaces it
        }
        if (current !== generation) {
            bitmap.close()
            return
        }
        hold({source: bitmap, width: bitmap.width, height: bitmap.height, close: () => bitmap.close()})
        draw()
    }

    watch(frameData, show)

    tryOnMounted(attach)

    tryOnBeforeUnmount(() => {
        generation++
        hold(null)
        surfaces.forEach((surface) => surface.cleanup())
        surfaces.clear()
        rendererBackend.value = 'unknown'
    })

    const backend = computed(() => (isJpeg.value ? 'jpeg' : rendererBackend.value))
    const label = computed(() => backendLabel(backend.value, dimensions.value))

    return {backend, label, redraw: draw}
}
