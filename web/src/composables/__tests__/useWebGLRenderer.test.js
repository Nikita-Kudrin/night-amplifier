import { describe, it, expect, vi, beforeEach } from 'vitest'
import { useWebGLRenderer } from '../useWebGLRenderer.js'

describe('useWebGLRenderer', () => {
    let canvas
    let gl

    beforeEach(() => {
        gl = {
            createShader: vi.fn(() => ({})),
            shaderSource: vi.fn(),
            compileShader: vi.fn(),
            getShaderParameter: vi.fn(() => true),
            createProgram: vi.fn(() => ({})),
            attachShader: vi.fn(),
            linkProgram: vi.fn(),
            getProgramParameter: vi.fn(() => true),
            createBuffer: vi.fn(() => ({})),
            bindBuffer: vi.fn(),
            bufferData: vi.fn(),
            createTexture: vi.fn(() => ({})),
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
            getParameter: vi.fn(() => 'Mock WebGL2'),
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
        canvas = {
            getContext: vi.fn((type) => {
                if (type === 'webgl2' || type === 'webgl' || type === 'experimental-webgl') return gl
                return null
            }),
            width: 0,
            height: 0,
        }
    })

    it('initializes correctly', () => {
        const { init, backend, isInitialized } = useWebGLRenderer()
        const consoleSpy = vi.spyOn(console, 'log').mockImplementation(() => {})
        const success = init(canvas)
        expect(success).toBe(true)
        expect(backend.value).toBe('webgl2-8bit')
        expect(isInitialized()).toBe(true)
        consoleSpy.mockRestore()
    })

    it('renders Uint8Array correctly', () => {
        const { init, render } = useWebGLRenderer()
        const consoleSpy = vi.spyOn(console, 'log').mockImplementation(() => {})
        init(canvas)
        consoleSpy.mockRestore()

        const frameData = new Uint8Array(10 * 10 * 3)
        render(canvas, frameData, 10, 10)

        // WebGL texImage2D for Uint8Array (target, level, internalformat, width, height, border, format, type, source)
        expect(gl.texImage2D).toHaveBeenCalledWith(
            gl.TEXTURE_2D, 0, gl.RGB, 10, 10, 0, gl.RGB, gl.UNSIGNED_BYTE, frameData
        )
    })

    it('renders ImageBitmap correctly', () => {
        const { init, render } = useWebGLRenderer()
        const consoleSpy = vi.spyOn(console, 'log').mockImplementation(() => {})
        init(canvas)
        consoleSpy.mockRestore()

        // Mock ImageBitmap
        class MockImageBitmap {}
        global.ImageBitmap = MockImageBitmap

        const frameData = new MockImageBitmap()
        render(canvas, frameData, 10, 10)

        // WebGL texImage2D for ImageBitmap (target, level, internalformat, format, type, source)
        expect(gl.texImage2D).toHaveBeenCalledWith(
            gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, frameData
        )
        
        delete global.ImageBitmap
    })

    function initQuietly() {
        const renderer = useWebGLRenderer()
        const consoleSpy = vi.spyOn(console, 'log').mockImplementation(() => {})
        renderer.init(canvas)
        consoleSpy.mockRestore()
        return renderer
    }

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

        render(canvas, new Uint8Array(width * height * 3), width, height)

        expect(gl.lastError).toBe(0)
        expect(gl.uploads).toEqual([{width, height}])
    })

    it('sets the unpack alignment before the RGB upload', () => {
        const { render } = initQuietly()

        render(canvas, new Uint8Array(5 * 4 * 3), 5, 4)

        const alignCall = gl.pixelStorei.mock.invocationCallOrder[0]
        const uploadCall = gl.texImage2D.mock.invocationCallOrder[0]
        expect(gl.pixelStorei).toHaveBeenCalledWith(gl.UNPACK_ALIGNMENT, 1)
        expect(alignCall).toBeLessThan(uploadCall)
    })

    it('keeps showing frames when consecutive frames change between odd and even widths', () => {
        const { render } = initQuietly()

        for (const [width, height] of [[2539, 1440], [1440, 1440], [2539, 1440], [1081, 1080]]) {
            render(canvas, new Uint8Array(width * height * 3), width, height)
        }

        expect(gl.lastError).toBe(0)
        expect(gl.uploads).toHaveLength(4)
    })
})
