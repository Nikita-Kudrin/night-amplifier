import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { useWebGLRenderer } from '../useWebGLRenderer.js'
import { useCanvas2DRenderer } from '../useCanvas2DRenderer.js'

function createGl(type, {compiles}) {
    const gl = {
        type,
        createShader: vi.fn(() => ({})),
        shaderSource: vi.fn(),
        compileShader: vi.fn(),
        getShaderParameter: vi.fn(() => compiles),
        getShaderInfoLog: vi.fn(() => 'unsupported'),
        deleteShader: vi.fn(),
        createProgram: vi.fn(() => ({})),
        attachShader: vi.fn(),
        linkProgram: vi.fn(),
        getProgramParameter: vi.fn(() => true),
        deleteProgram: vi.fn(),
        createBuffer: vi.fn(() => ({})),
        deleteBuffer: vi.fn(),
        bindBuffer: vi.fn(),
        bufferData: vi.fn(),
        createTexture: vi.fn(() => ({})),
        deleteTexture: vi.fn(),
        bindTexture: vi.fn(),
        texParameteri: vi.fn(),
        pixelStorei: vi.fn((pname, value) => {
            if (pname === gl.UNPACK_ALIGNMENT) gl.unpackAlignment = value
        }),
        // Validates the pixel buffer the way the WebGL spec does, so a row-padding
        // mismatch fails here as it does in a browser (INVALID_OPERATION, no upload).
        texImage2D: vi.fn((...args) => {
            if (args.length !== 9) return
            const [, , , width, height, , , , pixels] = args
            const rowBytes = width * 3
            const padded = Math.ceil(rowBytes / gl.unpackAlignment) * gl.unpackAlignment
            const required = height > 0 ? (height - 1) * padded + rowBytes : 0
            if (pixels.byteLength < required) gl.lastError = gl.INVALID_OPERATION
            else gl.uploads.push({width, height})
        }),
        unpackAlignment: 4,
        lastError: 0,
        uploads: [],
        viewport: vi.fn(),
        clearColor: vi.fn(),
        clear: vi.fn(),
        useProgram: vi.fn(),
        getAttribLocation: vi.fn(() => 1),
        enableVertexAttribArray: vi.fn(),
        vertexAttribPointer: vi.fn(),
        getUniformLocation: vi.fn(() => ({})),
        uniform1i: vi.fn(),
        drawArrays: vi.fn(),
        getParameter: vi.fn(() => 'Mock GPU'),
        loseContext: vi.fn(),
        getExtension: vi.fn((name) => (name === 'WEBGL_lose_context' ? {loseContext: gl.loseContext} : null)),
        VERTEX_SHADER: 1,
        FRAGMENT_SHADER: 2,
        COMPILE_STATUS: 3,
        LINK_STATUS: 4,
        ARRAY_BUFFER: 5,
        STATIC_DRAW: 6,
        TEXTURE_2D: 7,
        TEXTURE_WRAP_S: 8,
        TEXTURE_WRAP_T: 9,
        CLAMP_TO_EDGE: 10,
        TEXTURE_MIN_FILTER: 11,
        TEXTURE_MAG_FILTER: 12,
        LINEAR: 13,
        RGB: 14,
        RGBA: 15,
        UNSIGNED_BYTE: 16,
        COLOR_BUFFER_BIT: 17,
        FLOAT: 18,
        TRIANGLE_STRIP: 19,
        UNPACK_ALIGNMENT: 20,
        INVALID_OPERATION: 21,
    }
    return gl
}

function create2d() {
    return {
        type: '2d',
        createImageData: vi.fn((width, height) => ({data: new Uint8ClampedArray(4 * width * height), width, height})),
        putImageData: vi.fn(),
        drawImage: vi.fn(),
    }
}

/**
 * Canvases as a browser hands out contexts: the first type asked for is the canvas's for
 * life, and any other type answers null. `gpu` names the WebGL types the GPU offers and
 * the ones whose shaders compile.
 */
function installCanvasContexts(gpu = {}) {
    const {available = ['webgl2', 'webgl'], compiling = available} = gpu
    const contexts = new WeakMap()
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(function (type) {
        const held = contexts.get(this)
        if (held) return held.type === type ? held : null
        const family = type === 'experimental-webgl' ? 'webgl' : type
        let ctx = null
        if (family === '2d') ctx = create2d()
        else if (available.includes(family)) ctx = createGl(type, {compiles: compiling.includes(family)})
        if (ctx) contexts.set(this, ctx)
        return ctx
    })
    return {contextOf: (canvas) => contexts.get(canvas) ?? null}
}

function quietly(fn) {
    const spies = ['log', 'warn', 'error'].map((level) => vi.spyOn(console, level).mockImplementation(() => {}))
    try {
        return fn()
    } finally {
        spies.forEach((spy) => spy.mockRestore())
    }
}

describe('useWebGLRenderer', () => {
    let canvas
    let contextOf

    beforeEach(() => {
        canvas = document.createElement('canvas')
        ;({contextOf} = installCanvasContexts())
    })

    afterEach(() => vi.restoreAllMocks())

    function initQuietly(options) {
        const renderer = useWebGLRenderer(options)
        quietly(() => renderer.init(canvas))
        return renderer
    }

    it('initializes correctly', () => {
        const { backend, isInitialized } = initQuietly()
        expect(backend.value).toBe('webgl2-8bit')
        expect(isInitialized()).toBe(true)
        expect(contextOf(canvas).type).toBe('webgl2')
    })

    it('renders Uint8Array correctly', () => {
        const { render } = initQuietly()
        const gl = contextOf(canvas)

        const frameData = new Uint8Array(10 * 10 * 3)
        render(canvas, frameData, 10, 10)

        // WebGL texImage2D for Uint8Array (target, level, internalformat, width, height, border, format, type, source)
        expect(gl.texImage2D).toHaveBeenCalledWith(
            gl.TEXTURE_2D, 0, gl.RGB, 10, 10, 0, gl.RGB, gl.UNSIGNED_BYTE, frameData
        )
    })

    it('renders ImageBitmap correctly', () => {
        const { render } = initQuietly()
        const gl = contextOf(canvas)

        class MockImageBitmap {}
        vi.stubGlobal('ImageBitmap', MockImageBitmap)

        const frameData = new MockImageBitmap()
        render(canvas, frameData, 10, 10)

        // WebGL texImage2D for ImageBitmap (target, level, internalformat, format, type, source)
        expect(gl.texImage2D).toHaveBeenCalledWith(
            gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, frameData
        )
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
        const gl = contextOf(canvas)

        render(canvas, new Uint8Array(width * height * 3), width, height)

        expect(gl.lastError).toBe(0)
        expect(gl.uploads).toEqual([{width, height}])
    })

    it('sets the unpack alignment before the RGB upload', () => {
        const { render } = initQuietly()
        const gl = contextOf(canvas)

        render(canvas, new Uint8Array(5 * 4 * 3), 5, 4)

        const alignCall = gl.pixelStorei.mock.invocationCallOrder[0]
        const uploadCall = gl.texImage2D.mock.invocationCallOrder[0]
        expect(gl.pixelStorei).toHaveBeenCalledWith(gl.UNPACK_ALIGNMENT, 1)
        expect(alignCall).toBeLessThan(uploadCall)
    })

    it('keeps showing frames when consecutive frames change between odd and even widths', () => {
        const { render } = initQuietly()
        const gl = contextOf(canvas)

        for (const [width, height] of [[2539, 1440], [1440, 1440], [2539, 1440], [1081, 1080]]) {
            render(canvas, new Uint8Array(width * height * 3), width, height)
        }

        expect(gl.lastError).toBe(0)
        expect(gl.uploads).toHaveLength(4)
    })

    describe('falling back', () => {
        it('takes WebGL1 when WebGL2 shaders fail to compile', () => {
            ;({contextOf} = installCanvasContexts({compiling: ['webgl']}))

            const { backend, render } = initQuietly()
            render(canvas, new Uint8Array(4 * 4 * 3), 4, 4)

            expect(backend.value).toBe('webgl1')
            expect(contextOf(canvas).type).toBe('webgl')
            expect(contextOf(canvas).uploads).toEqual([{width: 4, height: 4}])
        })

        it('leaves the canvas to Canvas2D when no WebGL program builds, and it draws', () => {
            ;({contextOf} = installCanvasContexts({compiling: []}))
            const webgl = useWebGLRenderer()
            const canvas2d = useCanvas2DRenderer()

            expect(quietly(() => webgl.init(canvas))).toBe(false)
            expect(webgl.backend.value).toBe('none')
            expect(quietly(() => canvas2d.init(canvas))).toBe(true)
            canvas2d.render(canvas, new Uint8Array(4 * 4 * 3), 4, 4)

            expect(contextOf(canvas).putImageData).toHaveBeenCalledTimes(1)
        })

        it('frees every probe context instead of waiting for the GC', () => {
            const probes = []
            const original = document.createElement.bind(document)
            vi.spyOn(document, 'createElement').mockImplementation((tag) => {
                const element = original(tag)
                if (tag === 'canvas') probes.push(element)
                return element
            })

            initQuietly()

            expect(probes).toHaveLength(1)
            expect(contextOf(probes[0]).loseContext).toHaveBeenCalled()
        })

        it('never links a program from a shader that failed to compile', () => {
            ;({contextOf} = installCanvasContexts({compiling: ['webgl']}))
            const createdProbes = []
            const original = document.createElement.bind(document)
            vi.spyOn(document, 'createElement').mockImplementation((tag) => {
                const element = original(tag)
                if (tag === 'canvas') createdProbes.push(element)
                return element
            })

            initQuietly()

            const failed = contextOf(createdProbes[0])
            expect(failed.type).toBe('webgl2')
            expect(failed.createProgram).not.toHaveBeenCalled()
            expect(failed.deleteShader).toHaveBeenCalled()
        })
    })

    describe('context loss', () => {
        it('stops drawing while lost, then rebuilds and asks for a redraw on restore', () => {
            const onRestored = vi.fn()
            const { render, isInitialized } = initQuietly({onRestored})
            const gl = contextOf(canvas)
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

            cleanup()
            quietly(() => canvas.dispatchEvent(new Event('webglcontextrestored')))

            expect(onRestored).not.toHaveBeenCalled()
        })
    })
})
