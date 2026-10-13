import { describe, it, expect, expectTypeOf, vi, beforeEach, afterEach } from 'vitest'
import { useWebGLRenderer } from '../useWebGLRenderer.js'
import type { WebGLRendererOptions } from '../useWebGLRenderer.js'
import { useCanvas2DRenderer } from '../useCanvas2DRenderer.js'
import { installCanvasContexts, quietly } from '../../test/renderStubs.js'

describe('useWebGLRenderer', () => {
    let canvas: HTMLCanvasElement
    let contexts: ReturnType<typeof installCanvasContexts>
    const glOf = (target: HTMLCanvasElement) => contexts.glOf(target)

    beforeEach(() => {
        canvas = document.createElement('canvas')
        contexts = installCanvasContexts()
    })

    afterEach(() => vi.restoreAllMocks())

    function initQuietly(options?: WebGLRendererOptions) {
        const renderer = useWebGLRenderer(options)
        quietly(() => renderer.init(canvas))
        return renderer
    }

    it('initializes correctly', () => {
        const { backend, isInitialized } = initQuietly()
        expect(backend.value).toBe('webgl2-8bit')
        expect(isInitialized()).toBe(true)
        expect(glOf(canvas).type).toBe('webgl2')
    })

    it('renders Uint8Array correctly', () => {
        const { render } = initQuietly()
        const gl = glOf(canvas)

        const frameData = new Uint8Array(10 * 10 * 3)
        render(canvas, frameData, 10, 10)

        // WebGL texImage2D for Uint8Array (target, level, internalformat, width, height, border, format, type, source)
        expect(gl.texImage2D).toHaveBeenCalledWith(
            gl.TEXTURE_2D, 0, gl.RGB, 10, 10, 0, gl.RGB, gl.UNSIGNED_BYTE, frameData
        )
    })

    it('renders ImageBitmap correctly', () => {
        const { render } = initQuietly()
        const gl = glOf(canvas)

        class MockImageBitmap {}
        vi.stubGlobal('ImageBitmap', MockImageBitmap)

        const frameData = new MockImageBitmap() as unknown as ImageBitmap
        render(canvas, frameData, 10, 10)

        // WebGL texImage2D for ImageBitmap (target, level, internalformat, format, type, source)
        expect(gl.texImage2D).toHaveBeenCalledWith(
            gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, frameData
        )
    })

    // A JPEG blob is decoded first (`useFrameRenderer`); uploaded as pixels it drew nothing.
    it('takes RGB8 pixels or a decoded bitmap, never a JPEG blob', () => {
        const { render } = useWebGLRenderer()
        expectTypeOf(render).parameter(1).toEqualTypeOf<Uint8Array | ImageBitmap | null>()
        expectTypeOf(render).parameter(1).not.toExtend<Blob>()
    })

    // Every width the server can send, including odd ones: IMX464 (2712x1538) on the
    // 1440 tier comes out 2539x1440. With the default 4-byte row alignment those
    // frames were rejected and the screen froze on the previous one.
    it.each([
        [2539, 1440],
        [1, 1],
        [2, 3],
        [3, 2],
        [1079, 1080],
        [1440, 1440],
        [2712, 1538],
    ])('uploads every %ix%i RGB frame without a row-alignment error', (width, height) => {
        const { render } = initQuietly()
        const gl = glOf(canvas)

        render(canvas, new Uint8Array(width * height * 3), width, height)

        expect(gl.lastError).toBe(0)
        expect(gl.uploads).toEqual([{width, height}])
    })

    it('sets the unpack alignment before the RGB upload', () => {
        const { render } = initQuietly()
        const gl = glOf(canvas)

        render(canvas, new Uint8Array(5 * 4 * 3), 5, 4)

        const alignCall = gl.pixelStorei.mock.invocationCallOrder[0]
        const uploadCall = gl.texImage2D.mock.invocationCallOrder[0]
        expect(gl.pixelStorei).toHaveBeenCalledWith(gl.UNPACK_ALIGNMENT, 1)
        expect(alignCall).toBeLessThan(uploadCall)
    })

    it('keeps showing frames when consecutive frames change between odd and even widths', () => {
        const { render } = initQuietly()
        const gl = glOf(canvas)

        for (const [width, height] of [[2539, 1440], [1440, 1440], [2539, 1440], [1081, 1080]]) {
            render(canvas, new Uint8Array(width * height * 3), width, height)
        }

        expect(gl.lastError).toBe(0)
        expect(gl.uploads).toHaveLength(4)
    })

    it('looks the program\'s locations up once, not every frame', () => {
        const { render } = initQuietly()
        const gl = glOf(canvas)
        const lookups = () => gl.getAttribLocation.mock.calls.length + gl.getUniformLocation.mock.calls.length

        render(canvas, new Uint8Array(4 * 4 * 3), 4, 4)
        const afterFirst = lookups()
        render(canvas, new Uint8Array(4 * 4 * 3), 4, 4)

        expect(afterFirst).toBe(3)
        expect(lookups()).toBe(afterFirst)
    })

    describe('falling back', () => {
        it('takes WebGL1 when WebGL2 shaders fail to compile', () => {
            contexts = installCanvasContexts({compiling: ['webgl']})

            const { backend, render } = initQuietly()
            render(canvas, new Uint8Array(4 * 4 * 3), 4, 4)

            expect(backend.value).toBe('webgl1')
            expect(glOf(canvas).type).toBe('webgl')
            expect(glOf(canvas).uploads).toEqual([{width: 4, height: 4}])
        })

        it('leaves the canvas to Canvas2D when no WebGL program builds, and it draws', () => {
            contexts = installCanvasContexts({compiling: []})
            const webgl = useWebGLRenderer()
            const canvas2d = useCanvas2DRenderer()

            expect(quietly(() => webgl.init(canvas))).toBe(false)
            expect(webgl.backend.value).toBe('none')
            expect(quietly(() => canvas2d.init(canvas))).toBe(true)
            canvas2d.render(canvas, new Uint8Array(4 * 4 * 3), 4, 4)

            expect(contexts.context2dOf(canvas).putImageData).toHaveBeenCalledTimes(1)
        })

        it('frees every probe context instead of waiting for the GC', () => {
            const probes: HTMLCanvasElement[] = []
            const original = document.createElement.bind(document)
            vi.spyOn(document, 'createElement').mockImplementation((tag: string) => {
                const element = original(tag)
                if (element instanceof HTMLCanvasElement) probes.push(element)
                return element
            })

            initQuietly()

            expect(probes).toHaveLength(1)
            expect(glOf(probes[0]).loseContext).toHaveBeenCalled()
        })

        it('never links a program from a shader that failed to compile', () => {
            contexts = installCanvasContexts({compiling: ['webgl']})
            const createdProbes: HTMLCanvasElement[] = []
            const original = document.createElement.bind(document)
            vi.spyOn(document, 'createElement').mockImplementation((tag: string) => {
                const element = original(tag)
                if (element instanceof HTMLCanvasElement) createdProbes.push(element)
                return element
            })

            initQuietly()

            const failed = glOf(createdProbes[0])
            expect(failed.type).toBe('webgl2')
            expect(failed.createProgram).not.toHaveBeenCalled()
            expect(failed.deleteShader).toHaveBeenCalled()
        })
    })

    describe('context loss', () => {
        it('stops drawing while lost, then rebuilds and asks for a redraw on restore', () => {
            const onRestored = vi.fn()
            const { render, isInitialized } = initQuietly({onRestored})
            const gl = glOf(canvas)
            const programsBefore = gl.createProgram.mock.calls.length

            const lost = new Event('webglcontextlost', {cancelable: true})
            quietly(() => canvas.dispatchEvent(lost))
            render(canvas, new Uint8Array(4 * 4 * 3), 4, 4)

            expect(lost.defaultPrevented).toBe(true)
            expect(gl.uploads).toHaveLength(0)
            expect(isInitialized()).toBe(true)

            quietly(() => canvas.dispatchEvent(new Event('webglcontextrestored')))
            render(canvas, new Uint8Array(4 * 4 * 3), 4, 4)

            expect(gl.createProgram.mock.calls.length).toBe(programsBefore + 1)
            expect(onRestored).toHaveBeenCalledTimes(1)
            expect(gl.uploads).toEqual([{width: 4, height: 4}])
        })

        it('stops listening once cleaned up', () => {
            const onRestored = vi.fn()
            const { cleanup } = initQuietly({onRestored})
            const gl = glOf(canvas)
            const programsBefore = gl.createProgram.mock.calls.length

            cleanup()
            quietly(() => canvas.dispatchEvent(new Event('webglcontextrestored')))

            expect(onRestored).not.toHaveBeenCalled()
            expect(gl.createProgram.mock.calls.length).toBe(programsBefore)
        })

        // Browsers cap live contexts (~16); a binoview toggle makes new canvases each time.
        it('releases the canvas\'s context on cleanup, not on a context loss', () => {
            const { cleanup } = initQuietly()
            const gl = glOf(canvas)

            quietly(() => canvas.dispatchEvent(new Event('webglcontextlost', {cancelable: true})))
            expect(gl.loseContext).not.toHaveBeenCalled()

            cleanup()
            expect(gl.loseContext).toHaveBeenCalledTimes(1)
        })

        it('reports whether it drew', () => {
            const { render, cleanup } = initQuietly()
            const frame = new Uint8Array(4 * 4 * 3)
            expect(render(canvas, frame, 4, 4)).toBe(true)

            quietly(() => canvas.dispatchEvent(new Event('webglcontextlost', {cancelable: true})))
            expect(render(canvas, frame, 4, 4)).toBe(false)

            cleanup()
            expect(render(canvas, frame, 4, 4)).toBe(false)
        })
    })
})
