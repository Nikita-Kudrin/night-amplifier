import { describe, it, expect, expectTypeOf, vi, beforeEach, afterEach } from 'vitest'
import { CONTEXT_RESTORE_TIMEOUT_MS, useWebGLRenderer } from '../useWebGLRenderer'
import type { WebGLRendererOptions } from '../useWebGLRenderer'
import { useCanvas2DRenderer } from '../useCanvas2DRenderer'
import { installCanvasContexts, quietly } from '../../test/renderStubs'

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

        expect(gl.errors).toEqual([])
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

        expect(gl.errors).toEqual([])
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

    // A new storage a frame reallocated a 12-27 MB texture each time on the kiosk.
    describe('texture storage', () => {
        class MockImageBitmap {
            width: number
            height: number
            constructor(width: number, height: number) {
                this.width = width
                this.height = height
            }
        }
        const bitmapOf = (width: number, height: number) => new MockImageBitmap(width, height) as unknown as ImageBitmap

        it('allocates once and updates it in place for frames of the same size', () => {
            const { render } = initQuietly()
            const gl = glOf(canvas)
            const first = new Uint8Array(5 * 4 * 3)
            const second = new Uint8Array(5 * 4 * 3)

            render(canvas, first, 5, 4)
            render(canvas, second, 5, 4)

            expect(gl.texImage2D).toHaveBeenCalledTimes(1)
            expect(gl.texSubImage2D).toHaveBeenCalledWith(
                gl.TEXTURE_2D, 0, 0, 0, 5, 4, gl.RGB, gl.UNSIGNED_BYTE, second
            )
            expect(gl.errors).toEqual([])
            expect(gl.uploads).toEqual([{width: 5, height: 4}, {width: 5, height: 4}])
        })

        it('allocates again for a new size or format, and reuses each in turn', () => {
            vi.stubGlobal('ImageBitmap', MockImageBitmap)
            const { render } = initQuietly()
            const gl = glOf(canvas)

            render(canvas, new Uint8Array(4 * 4 * 3), 4, 4)
            render(canvas, new Uint8Array(6 * 4 * 3), 6, 4)
            render(canvas, bitmapOf(6, 4), 6, 4)
            render(canvas, bitmapOf(6, 4), 6, 4)
            render(canvas, new Uint8Array(6 * 4 * 3), 6, 4)

            expect(gl.texImage2D).toHaveBeenCalledTimes(4)
            expect(gl.texSubImage2D).toHaveBeenCalledTimes(1)
            expect(gl.texSubImage2D).toHaveBeenCalledWith(gl.TEXTURE_2D, 0, 0, 0, gl.RGBA, gl.UNSIGNED_BYTE, expect.any(MockImageBitmap))
        })

        // Out of GPU memory with the context kept: updating storage that was never made
        // fails too, and the view stayed black until the frame size changed.
        it('allocates again after an allocation failed, and checks only allocations', () => {
            const { render } = initQuietly()
            const gl = glOf(canvas)

            gl.failNextAllocation = true
            render(canvas, new Uint8Array(4 * 4 * 3), 4, 4)
            render(canvas, new Uint8Array(4 * 4 * 3), 4, 4)
            render(canvas, new Uint8Array(4 * 4 * 3), 4, 4)

            expect(gl.errors).toEqual([gl.OUT_OF_MEMORY])
            expect(gl.texImage2D).toHaveBeenCalledTimes(2)
            expect(gl.texSubImage2D).toHaveBeenCalledTimes(1)
            expect(gl.uploads).toEqual([{width: 4, height: 4}, {width: 4, height: 4}])
            expect(gl.getError).toHaveBeenCalledTimes(2)
        })

        it('allocates the texture a restored context made, never updating the lost one', () => {
            const { render } = initQuietly()
            const gl = glOf(canvas)
            render(canvas, new Uint8Array(4 * 4 * 3), 4, 4)

            quietly(() => canvas.dispatchEvent(new Event('webglcontextlost', {cancelable: true})))
            quietly(() => canvas.dispatchEvent(new Event('webglcontextrestored')))
            render(canvas, new Uint8Array(4 * 4 * 3), 4, 4)

            expect(gl.texImage2D).toHaveBeenCalledTimes(2)
            expect(gl.texSubImage2D).not.toHaveBeenCalled()
        })
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

        // A browser that blocked WebGL for the page after GPU crashes never restores it.
        describe('a context that never comes back', () => {
            beforeEach(() => vi.useFakeTimers())
            afterEach(() => vi.useRealTimers())

            const lose = () => quietly(() => canvas.dispatchEvent(new Event('webglcontextlost', {cancelable: true})))

            it('says so once the restore is overdue', () => {
                const onLostForGood = vi.fn()
                initQuietly({onLostForGood})

                lose()
                vi.advanceTimersByTime(CONTEXT_RESTORE_TIMEOUT_MS - 1)
                expect(onLostForGood).not.toHaveBeenCalled()
                quietly(() => vi.advanceTimersByTime(1))

                expect(onLostForGood).toHaveBeenCalledTimes(1)
            })

            it('says nothing of one restored in time, or of a renderer cleaned up', () => {
                const onLostForGood = vi.fn()
                initQuietly({onLostForGood})
                lose()
                quietly(() => canvas.dispatchEvent(new Event('webglcontextrestored')))
                const other = document.createElement('canvas')
                const cleanedUp = useWebGLRenderer({onLostForGood})
                quietly(() => cleanedUp.init(other))
                quietly(() => other.dispatchEvent(new Event('webglcontextlost', {cancelable: true})))
                cleanedUp.cleanup()

                vi.advanceTimersByTime(CONTEXT_RESTORE_TIMEOUT_MS)

                expect(onLostForGood).not.toHaveBeenCalled()
                expect(vi.getTimerCount()).toBe(0)
            })

            describe('while the page is hidden', () => {
                const setVisibility = (state: DocumentVisibilityState) => {
                    Object.defineProperty(document, 'visibilityState', {value: state, configurable: true})
                    document.dispatchEvent(new Event('visibilitychange'))
                }
                afterEach(() => setVisibility('visible'))

                // A phone's browser drops a background tab's context and restores it only
                // once the tab is shown again: time hidden is no restore that failed.
                it('does not count the time the page spent hidden', () => {
                    const onLostForGood = vi.fn()
                    initQuietly({onLostForGood})
                    setVisibility('hidden')

                    lose()
                    quietly(() => vi.advanceTimersByTime(3 * CONTEXT_RESTORE_TIMEOUT_MS))
                    setVisibility('visible')
                    quietly(() => canvas.dispatchEvent(new Event('webglcontextrestored')))
                    vi.advanceTimersByTime(CONTEXT_RESTORE_TIMEOUT_MS)

                    expect(onLostForGood).not.toHaveBeenCalled()
                    expect(vi.getTimerCount()).toBe(0)
                })

                it('gives a context lost while shown the whole deadline again once shown', () => {
                    const onLostForGood = vi.fn()
                    initQuietly({onLostForGood})

                    lose()
                    vi.advanceTimersByTime(CONTEXT_RESTORE_TIMEOUT_MS - 1000)
                    setVisibility('hidden')
                    vi.advanceTimersByTime(3 * CONTEXT_RESTORE_TIMEOUT_MS)
                    setVisibility('visible')
                    vi.advanceTimersByTime(CONTEXT_RESTORE_TIMEOUT_MS - 1)
                    expect(onLostForGood).not.toHaveBeenCalled()
                    quietly(() => vi.advanceTimersByTime(1))

                    expect(onLostForGood).toHaveBeenCalledTimes(1)
                    // Said once: a later visibility change starts nothing.
                    setVisibility('hidden')
                    setVisibility('visible')
                    expect(vi.getTimerCount()).toBe(0)
                })

                it('stops listening for visibility once restored or cleaned up', () => {
                    const removed = vi.spyOn(document, 'removeEventListener')
                    const renderer = initQuietly()
                    lose()
                    quietly(() => canvas.dispatchEvent(new Event('webglcontextrestored')))
                    expect(removed).toHaveBeenCalledWith('visibilitychange', expect.any(Function))

                    removed.mockClear()
                    lose()
                    renderer.cleanup()
                    expect(removed).toHaveBeenCalledWith('visibilitychange', expect.any(Function))
                })
            })
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
