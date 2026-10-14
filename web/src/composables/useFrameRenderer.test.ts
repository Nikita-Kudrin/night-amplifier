import {defineComponent, h, nextTick, ref, shallowRef} from 'vue'
import {mount} from '@vue/test-utils'
import {BACKEND_LABELS, backendLabel, useFrameRenderer} from './useFrameRenderer'
import type {FrameRenderer, FrameRendererOptions} from './useFrameRenderer'
import type {FrameData, FrameSize} from './useImageStream'
import {installCanvasContexts, quietly} from '../test/renderStubs'

/** A JPEG as the stream hands it on, carrying the size its fake decode reports. */
type SizedJpeg = Blob & FrameSize

function jpegOf(width: number, height: number): SizedJpeg {
    return Object.assign(new Blob([], {type: 'image/jpeg'}), {width, height})
}

class FakeImageBitmap {}
type FakeBitmap = FakeImageBitmap & FrameSize & {close: ReturnType<typeof vi.fn>}

interface PendingDecode {
    blob: SizedJpeg
    resolve(): void
    reject(error: Error): void
}

/** A JPEG decode resolved by hand, so a test decides when each one finishes. */
function installDecoder() {
    const pending: PendingDecode[] = []
    const bitmaps: FakeBitmap[] = []
    vi.stubGlobal('ImageBitmap', FakeImageBitmap)
    vi.stubGlobal('createImageBitmap', vi.fn((blob: SizedJpeg) => new Promise<FakeBitmap>((resolve, reject) => {
        pending.push({
            blob,
            resolve() {
                const bitmap = Object.assign(new FakeImageBitmap(), {width: blob.width, height: blob.height, close: vi.fn()})
                bitmaps.push(bitmap)
                resolve(bitmap)
            },
            reject,
        })
    })))
    return {pending, bitmaps}
}

interface MountOptions {
    count?: number
    isJpeg?: boolean
    onSizeChange?: FrameRendererOptions['onSizeChange']
}

/** Mounts `count` canvases fed by one stream, as the live view (1) and binoview (2) do. */
function mountRenderer({count = 1, isJpeg = false, onSizeChange}: MountOptions = {}) {
    const stream = {
        frameData: shallowRef<FrameData | null>(null),
        dimensions: ref<FrameSize>({width: 0, height: 0}),
        isJpeg: ref(isJpeg),
    }
    const canvasCount = ref(count)
    const canvases: HTMLCanvasElement[] = []
    let renderer: FrameRenderer | undefined
    const Host = defineComponent({
        setup() {
            renderer = useFrameRenderer({...stream, canvases: () => canvases.slice(0, canvasCount.value), onSizeChange})
            return () => h('div', Array.from({length: canvasCount.value}, (_, i) =>
                h('canvas', {key: i, ref: (el) => { if (el instanceof HTMLCanvasElement) canvases[i] = el }})))
        },
    })
    const wrapper = quietly(() => mount(Host))
    return {
        wrapper,
        stream,
        canvases,
        canvasCount,
        renderer: () => {
            if (!renderer) throw new Error('the host never set up')
            return renderer
        },
        async showRgb(width: number, height: number) {
            stream.dimensions.value = {width, height}
            stream.frameData.value = new Uint8Array(width * height * 3)
            await nextTick()
        },
        async showJpeg(width: number, height: number) {
            stream.dimensions.value = {width, height}
            stream.frameData.value = jpegOf(width, height)
            await nextTick()
        },
    }
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0))

describe('backendLabel', () => {
    it('names each backend with the frame size', () => {
        expect(backendLabel('webgl2-8bit', {width: 2539, height: 1440})).toBe('WebGL2 8-bit (2539x1440)')
        expect(backendLabel('jpeg', {width: 1440, height: 1440})).toBe('Dynamic JPEG (1440x1440)')
        expect(backendLabel('canvas2d', {width: 0, height: 0})).toBe('Canvas 2D')
    })

    it('gives no size for a missing renderer, and an unknown backend its own name', () => {
        expect(backendLabel('none', {width: 10, height: 10})).toBe('No renderer')
        expect(backendLabel('unknown')).toBe('...')
        expect(backendLabel('webgpu', {})).toBe('webgpu')
    })

    it('has no label for a backend no renderer reports', () => {
        expect(Object.keys(BACKEND_LABELS)).not.toContain('webgl2-16bit')
    })
})

describe('useFrameRenderer', () => {
    afterEach(() => vi.restoreAllMocks())

    describe('picking a backend', () => {
        it('takes WebGL2 when it works', async () => {
            const {glOf} = installCanvasContexts()
            const view = mountRenderer()
            await view.showRgb(4, 4)

            expect(view.renderer().backend.value).toBe('webgl2-8bit')
            expect(glOf(view.canvases[0]).uploads).toEqual([{width: 4, height: 4}])
            expect(view.renderer().label.value).toBe('WebGL2 8-bit (4x4)')
        })

        it('falls back to WebGL1 when WebGL2 shaders fail', async () => {
            const {glOf} = installCanvasContexts({compiling: ['webgl']})
            const view = mountRenderer()
            await view.showRgb(4, 4)

            expect(view.renderer().backend.value).toBe('webgl1')
            expect(glOf(view.canvases[0]).uploads).toEqual([{width: 4, height: 4}])
        })

        // The canvas hands out one context type for life: had WebGL2 been taken on it
        // before its shaders failed, Canvas 2D would get null and the view stay blank.
        it('draws a Canvas 2D frame when no WebGL shader compiles', async () => {
            const {context2dOf} = installCanvasContexts({compiling: []})
            const view = mountRenderer()
            await quietly(() => view.showRgb(2, 1))

            const ctx = context2dOf(view.canvases[0])
            expect(view.renderer().backend.value).toBe('canvas2d')
            expect(ctx.type).toBe('2d')
            expect(ctx.putImageData).toHaveBeenCalledTimes(1)
            expect(ctx.putImageData.mock.calls[0][0].data.length).toBe(2 * 1 * 4)
        })

        it('says so when the canvas offers no context at all', async () => {
            vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null)
            const view = mountRenderer()
            await quietly(() => view.showRgb(2, 2))

            expect(view.renderer().backend.value).toBe('none')
            expect(view.renderer().label.value).toBe('No renderer')
        })
    })

    describe('several canvases', () => {
        it('decodes a JPEG once and draws it on every eye', async () => {
            const {glOf} = installCanvasContexts()
            const decoder = installDecoder()
            const view = mountRenderer({count: 2, isJpeg: true})
            await view.showJpeg(8, 6)

            decoder.pending[0].resolve()
            await flush()

            expect(window.createImageBitmap).toHaveBeenCalledTimes(1)
            for (const canvas of view.canvases) {
                expect(glOf(canvas).texImage2D).toHaveBeenCalledWith(
                    expect.anything(), 0, expect.anything(), expect.anything(), expect.anything(), decoder.bitmaps[0])
            }
            expect(view.renderer().label.value).toBe('Dynamic JPEG (8x6)')
        })

        it('draws the frame it holds on a canvas that appears, and frees the one that went', async () => {
            const {glOf} = installCanvasContexts()
            const view = mountRenderer({count: 2})
            await view.showRgb(4, 4)
            const [left, right] = view.canvases
            const rightContext = glOf(right)

            view.canvasCount.value = 1
            await nextTick()
            view.renderer().redraw()

            expect(glOf(left).uploads).toHaveLength(2)
            expect(rightContext.uploads).toHaveLength(1)
            expect(rightContext.deleteTexture).toHaveBeenCalled()
            expect(rightContext.loseContext).toHaveBeenCalled()
            expect(glOf(left).loseContext).not.toHaveBeenCalled()
        })
    })

    describe('JPEG decodes', () => {
        // A phone decoding a 1440p JPEG slower than Focus mode's frame rate: each decode
        // finishes after the next frame arrived. Dropping those froze the view.
        it('keeps drawing when every decode outlasts the gap to the next frame', async () => {
            const {glOf} = installCanvasContexts()
            const decoder = installDecoder()
            const view = mountRenderer({isJpeg: true})
            await view.showJpeg(10, 10)
            for (let i = 1; i < 6; i++) {
                await view.showJpeg(10, 10)
                decoder.pending[i - 1].resolve()
                await flush()
            }

            const drawn = glOf(view.canvases[0]).texImage2D.mock.calls.map((call) => call.at(-1))
            expect(drawn).toEqual(decoder.bitmaps)
        })

        it('decodes only the newest of the frames that arrived during a decode, in order', async () => {
            const {glOf} = installCanvasContexts()
            const decoder = installDecoder()
            const view = mountRenderer({isJpeg: true})
            await view.showJpeg(10, 10)
            await view.showJpeg(20, 20)
            await view.showJpeg(30, 30)
            expect(window.createImageBitmap).toHaveBeenCalledTimes(1)

            decoder.pending[0].resolve()
            await flush()
            expect(decoder.pending.map((decode) => decode.blob.width)).toEqual([10, 30])
            decoder.pending[1].resolve()
            await flush()

            const drawn = glOf(view.canvases[0]).texImage2D.mock.calls.map((call) => call.at(-1))
            expect(drawn).toEqual(decoder.bitmaps)
        })

        it('never paints a decode over an RGB frame drawn after it started', async () => {
            const {glOf} = installCanvasContexts()
            const decoder = installDecoder()
            const view = mountRenderer({isJpeg: true})
            await view.showJpeg(10, 10)
            view.stream.isJpeg.value = false
            await view.showRgb(4, 4)

            decoder.pending[0].resolve()
            await flush()

            const gl = glOf(view.canvases[0])
            expect(gl.texImage2D).toHaveBeenCalledTimes(1)
            expect(gl.uploads).toEqual([{width: 4, height: 4}])
            expect(decoder.bitmaps[0].close).toHaveBeenCalled()
        })

        it('closes the bitmap it held once the next frame is drawn', async () => {
            installCanvasContexts()
            const decoder = installDecoder()
            const view = mountRenderer({isJpeg: true})
            await view.showJpeg(10, 10)
            decoder.pending[0].resolve()
            await flush()
            await view.showJpeg(10, 10)
            decoder.pending[1].resolve()
            await flush()

            expect(decoder.bitmaps[0].close).toHaveBeenCalled()
            expect(decoder.bitmaps[1].close).not.toHaveBeenCalled()
        })

        it('keeps the last picture when a frame cannot be decoded', async () => {
            const {glOf} = installCanvasContexts()
            const decoder = installDecoder()
            const view = mountRenderer({isJpeg: true})
            await view.showJpeg(10, 10)
            decoder.pending[0].resolve()
            await flush()
            await view.showJpeg(10, 10)
            decoder.pending[1].reject(new Error('corrupt'))
            await flush()

            expect(glOf(view.canvases[0]).texImage2D).toHaveBeenCalledTimes(1)
            expect(decoder.bitmaps[0].close).not.toHaveBeenCalled()
        })

        // `no_frame` after a camera switch: a decode of the old camera's last frame
        // finishing afterwards must not put it back up.
        it('drops a pending decode and the held picture when the frame is cleared', async () => {
            const {glOf} = installCanvasContexts()
            const decoder = installDecoder()
            const view = mountRenderer({isJpeg: true})
            await view.showJpeg(10, 10)
            decoder.pending[0].resolve()
            await flush()
            await view.showJpeg(10, 10)

            view.stream.frameData.value = null
            await nextTick()
            decoder.pending[1].resolve()
            await flush()
            view.renderer().redraw()

            expect(glOf(view.canvases[0]).texImage2D).toHaveBeenCalledTimes(1)
            expect(decoder.bitmaps[0].close).toHaveBeenCalled()
            expect(decoder.bitmaps[1].close).toHaveBeenCalled()
        })

        it('drops the frame waiting behind a decode when the frame is cleared', async () => {
            const {glOf} = installCanvasContexts()
            const decoder = installDecoder()
            const view = mountRenderer({isJpeg: true})
            await view.showJpeg(10, 10)
            await view.showJpeg(20, 20)

            view.stream.frameData.value = null
            await nextTick()
            decoder.pending[0].resolve()
            await flush()

            expect(window.createImageBitmap).toHaveBeenCalledTimes(1)
            expect(glOf(view.canvases[0]).texImage2D).not.toHaveBeenCalled()
            expect(decoder.bitmaps[0].close).toHaveBeenCalled()
        })
    })

    // The frame, not the stream's `isJpeg`, picks the path: a blob handed to a renderer
    // as pixels uploaded nothing (WebGL) or a blank picture (Canvas 2D).
    describe('what a frame is', () => {
        it('decodes a JPEG blob on a stream not marked JPEG, never drawing it as pixels', async () => {
            const {glOf} = installCanvasContexts()
            const decoder = installDecoder()
            const view = mountRenderer({isJpeg: false})
            await view.showJpeg(8, 6)
            decoder.pending[0].resolve()
            await flush()

            const gl = glOf(view.canvases[0])
            expect(gl.uploads).toEqual([])
            expect(gl.texImage2D.mock.calls.map((call) => call.at(-1))).toEqual(decoder.bitmaps)
        })

        it('draws RGB pixels on a JPEG stream as they are', async () => {
            const {glOf} = installCanvasContexts()
            installDecoder()
            const view = mountRenderer({isJpeg: true})
            await view.showRgb(4, 4)

            expect(window.createImageBitmap).not.toHaveBeenCalled()
            expect(glOf(view.canvases[0]).uploads).toEqual([{width: 4, height: 4}])
            expect(view.renderer().label.value).toBe('Dynamic JPEG (4x4)')
        })

        it('takes the stream\'s frame and hands out a backend the labels name', () => {
            expectTypeOf<FrameRendererOptions['frameData']['value']>().toEqualTypeOf<Uint8Array | Blob | null>()
            expectTypeOf<FrameRenderer['backend']['value']>().toEqualTypeOf<keyof typeof BACKEND_LABELS>()
        })
    })

    it('reports a new drawn size once, not every frame', async () => {
        installCanvasContexts()
        const onSizeChange = vi.fn()
        const view = mountRenderer({onSizeChange})

        await view.showRgb(4, 4)
        await view.showRgb(4, 4)
        await view.showRgb(6, 4)

        expect(onSizeChange.mock.calls).toEqual([[{width: 4, height: 4}], [{width: 6, height: 4}]])
    })

    // The guide arrow and comet ROI measure the canvas: a size "drawn" while the context
    // was lost never resized it, and the restore's redraw must still report it.
    it('reports a size first drawn after a lost context came back', async () => {
        installCanvasContexts()
        const onSizeChange = vi.fn()
        const view = mountRenderer({onSizeChange})
        await view.showRgb(4, 4)
        const canvas = view.canvases[0]

        quietly(() => canvas.dispatchEvent(new Event('webglcontextlost', {cancelable: true})))
        await view.showRgb(6, 4)
        expect(onSizeChange.mock.calls).toEqual([[{width: 4, height: 4}]])

        quietly(() => canvas.dispatchEvent(new Event('webglcontextrestored')))

        expect(canvas.width).toBe(6)
        expect(onSizeChange.mock.calls).toEqual([[{width: 4, height: 4}], [{width: 6, height: 4}]])
    })

    it('redraws the held frame when a lost context comes back', async () => {
        const {glOf} = installCanvasContexts()
        const view = mountRenderer()
        await view.showRgb(4, 4)
        const canvas = view.canvases[0]

        quietly(() => canvas.dispatchEvent(new Event('webglcontextlost', {cancelable: true})))
        quietly(() => canvas.dispatchEvent(new Event('webglcontextrestored')))

        expect(glOf(canvas).uploads).toHaveLength(2)
    })

    it('frees its renderers and the held bitmap on unmount', async () => {
        const {glOf} = installCanvasContexts()
        const decoder = installDecoder()
        const view = mountRenderer({isJpeg: true})
        await view.showJpeg(4, 4)
        decoder.pending[0].resolve()
        await flush()

        view.wrapper.unmount()

        expect(glOf(view.canvases[0]).deleteProgram).toHaveBeenCalled()
        expect(glOf(view.canvases[0]).loseContext).toHaveBeenCalled()
        expect(decoder.bitmaps[0].close).toHaveBeenCalled()
    })
})
