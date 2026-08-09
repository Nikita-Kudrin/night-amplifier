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
            texImage2D: vi.fn(),
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
})
