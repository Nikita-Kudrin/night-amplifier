import {computed, ref, watch} from 'vue'
import type {Ref} from 'vue'
import {tryOnBeforeUnmount, tryOnMounted} from './lifecycle'
import {useWebGLRenderer} from './useWebGLRenderer'
import type {RenderSource, RendererBackend} from './useWebGLRenderer'
import {useCanvas2DRenderer} from './useCanvas2DRenderer'
import type {FrameData, FrameSize} from './useImageStream'
import {formatResolution} from '../utils/format'

/** What the frame on screen is drawn by: a renderer, or `jpeg`, the stream's own. */
export type FrameBackend = RendererBackend | 'jpeg'

/** What the debug readout calls each backend; `jpeg` is the stream's, not a renderer's. */
export const BACKEND_LABELS = {
    jpeg: 'Dynamic JPEG',
    'webgl2-8bit': 'WebGL2 8-bit',
    webgl1: 'WebGL1',
    canvas2d: 'Canvas 2D',
    none: 'No renderer',
    unknown: '...',
} as const satisfies Record<FrameBackend, string>

/** `backend`'s label, with the frame size once there is one. */
export function backendLabel(backend: string, {width, height}: Partial<FrameSize> = {}) {
    const label = isFrameBackend(backend) ? BACKEND_LABELS[backend] : backend
    if (!((width ?? 0) > 0) || backend === 'none' || backend === 'unknown') return label
    return `${label} (${formatResolution(width, height)})`
}

function isFrameBackend(backend: string): backend is FrameBackend {
    return Object.hasOwn(BACKEND_LABELS, backend)
}

/** An image ready to draw, held for a redraw; a decoded JPEG's bitmap is closed when it is let go. */
interface HeldImage extends FrameSize {
    source: RenderSource
    close?: () => void
}

interface Surface {
    backend: () => RendererBackend
    draw: (image: HeldImage) => boolean
    cleanup: () => void
}

export interface FrameRendererOptions {
    /** The stream's frame: RGB8 bytes or a JPEG blob */
    frameData: Readonly<Ref<FrameData | null>>
    dimensions: Readonly<Ref<FrameSize>>
    isJpeg: Readonly<Ref<boolean>>
    /** The canvases on screen now */
    canvases: () => readonly (HTMLCanvasElement | null | undefined)[]
    /**
     * A frame of a new size was drawn: the canvas was resized, so whatever sits over it
     * re-measures. Not while no canvas drew (a lost context): the restore's redraw reports it.
     */
    onSizeChange?: (size: FrameSize) => void
}

/**
 * One canvas's renderer: WebGL2, then WebGL1, then Canvas 2D. `useWebGLRenderer` probes
 * each GL version on a scratch canvas first, so a failed one leaves this canvas free
 * for the next.
 */
function createSurface(canvas: HTMLCanvasElement, onRestored: () => void): Surface {
    const webgl = useWebGLRenderer({onRestored})
    const renderer = webgl.init(canvas) ? webgl : useCanvas2DRenderer()
    if (renderer !== webgl && !renderer.init(canvas)) {
        return {backend: () => 'none', draw: () => false, cleanup() {}}
    }
    return {
        backend: () => renderer.backend.value,
        /** Whether it drew: a lost GL context skips the frame, canvas resize included. */
        draw: (image: HeldImage) => renderer.render(canvas, image.source, image.width, image.height),
        cleanup: renderer.cleanup,
    }
}

/**
 * Draws an image stream's frames onto the canvases `canvases()` names: one for the live
 * view, two for a binoview eyepiece — a frame is decoded once and drawn on each.
 *
 * A JPEG decodes asynchronously, one at a time: frames arriving meanwhile replace each
 * other, and the newest is decoded next. Dropping every decode a newer frame overtook
 * froze the view whenever decoding was slower than the frame rate. The last image is
 * held, so a restored GL context or a newly shown canvas is drawn without waiting for
 * the next frame (an idle stream sends none); a null frame (`no_frame`, a cleared view)
 * drops it, and with it any decode under way.
 */
export function useFrameRenderer({frameData, dimensions, isJpeg, canvases, onSizeChange}: FrameRendererOptions) {
    const surfaces = new Map<HTMLCanvasElement, Surface>()
    const rendererBackend = ref<RendererBackend>('unknown')
    let image: HeldImage | null = null
    // Bumped by whatever outdates a decode under way: a clear, an RGB frame, unmount.
    let generation = 0
    let decoding = false
    let pendingJpeg: Blob | null = null
    let drawnSize: FrameSize | null = null

    function surfaceFor(canvas: HTMLCanvasElement) {
        let surface = surfaces.get(canvas)
        if (!surface) {
            surface = createSurface(canvas, draw)
            surfaces.set(canvas, surface)
        }
        return surface
    }

    /** Readies every canvas on screen, and frees the renderers of those that are gone. */
    function attach() {
        const targets = canvases().filter((canvas): canvas is HTMLCanvasElement => !!canvas)
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
        const held = image
        if (!held) return
        const drew = ready.map((surface) => surface.draw(held)).some(Boolean)
        if (!drew) return
        const {width, height} = held
        if (drawnSize?.width === width && drawnSize.height === height) return
        drawnSize = {width, height}
        onSizeChange?.(drawnSize)
    }

    function hold(next: HeldImage | null) {
        image?.close?.()
        image = next
    }

    /**
     * The frame says what it is: a JPEG blob is decoded, pixels are drawn as they are;
     * `isJpeg` only names the backend. Picked by `isJpeg`, a blob on a stream not marked
     * JPEG reached the renderer as pixels.
     */
    function show(frame: FrameData | null) {
        if (frame instanceof Blob) {
            pendingJpeg = frame
            if (!decoding) decodeLatest().catch((e: unknown) => console.error('[FrameRenderer] Drawing a frame failed:', e))
            return
        }
        generation++
        pendingJpeg = null
        if (!frame) {
            hold(null)
            return
        }
        hold({source: frame, ...dimensions.value})
        draw()
    }

    /** Decodes the newest pending JPEG until none is left; one decode in flight at most. */
    async function decodeLatest() {
        decoding = true
        try {
            while (pendingJpeg) {
                const frame = pendingJpeg
                pendingJpeg = null
                const current = generation
                const bitmap = await decode(frame)
                if (!bitmap) continue
                if (current !== generation) {
                    bitmap.close()
                    continue
                }
                hold({source: bitmap, width: bitmap.width, height: bitmap.height, close: () => bitmap.close()})
                draw()
            }
        } finally {
            decoding = false
        }
    }

    async function decode(frame: Blob) {
        try {
            return await window.createImageBitmap(frame)
        } catch {
            return null // not decodable; the next frame replaces it
        }
    }

    watch(frameData, show)

    tryOnMounted(attach)

    tryOnBeforeUnmount(() => {
        generation++
        pendingJpeg = null
        hold(null)
        surfaces.forEach((surface) => surface.cleanup())
        surfaces.clear()
        rendererBackend.value = 'unknown'
    })

    const backend = computed((): FrameBackend => (isJpeg.value ? 'jpeg' : rendererBackend.value))
    const label = computed(() => backendLabel(backend.value, dimensions.value))

    return {backend, label, redraw: draw}
}

export type FrameRenderer = ReturnType<typeof useFrameRenderer>
