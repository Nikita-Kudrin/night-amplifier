import {vi} from 'vitest'

/**
 * Canvas contexts as a browser hands them out, for renderer tests: real enum names, a
 * `texImage2D` that validates row padding the way the spec does, and one context type per
 * canvas for life. Restored by `vi.restoreAllMocks()`.
 */

type GlContextType = 'webgl2' | 'webgl' | 'experimental-webgl'

/** The `texImage2D` of an RGB8 buffer: target, level, internal format, size, border, format, type, pixels. */
type PixelUpload = [GLenum, GLint, GLint, GLsizei, GLsizei, GLint, GLenum, GLenum, ArrayBufferView]

function createGl(type: GlContextType, {compiles}: {compiles: boolean}) {
    const loseContext = vi.fn()
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
        pixelStorei: vi.fn((pname: GLenum, value: GLint) => {
            if (pname === gl.UNPACK_ALIGNMENT) gl.unpackAlignment = value
        }),
        // Validates the pixel buffer the way the WebGL spec does, so a row-padding
        // mismatch fails here as it does in a browser (INVALID_OPERATION, no upload).
        texImage2D: vi.fn((...args: unknown[]) => {
            if (args.length !== 9) return
            const [, , , width, height, , , , pixels] = args as PixelUpload
            const rowBytes = width * 3
            const padded = Math.ceil(rowBytes / gl.unpackAlignment) * gl.unpackAlignment
            const required = height > 0 ? (height - 1) * padded + rowBytes : 0
            if (pixels.byteLength < required) gl.lastError = gl.INVALID_OPERATION
            else gl.uploads.push({width, height})
        }),
        unpackAlignment: 4,
        lastError: 0,
        uploads: [] as {width: number; height: number}[],
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
        loseContext,
        getExtension: vi.fn((name: string) => (name === 'WEBGL_lose_context' ? {loseContext} : null)),
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
        type: '2d' as const,
        createImageData: vi.fn((width: number, height: number) =>
            ({data: new Uint8ClampedArray(4 * width * height), width, height})),
        putImageData: vi.fn<(imageData: ImageData, dx: number, dy: number) => void>(),
        drawImage: vi.fn<(image: CanvasImageSource, dx: number, dy: number) => void>(),
    }
}

/** A WebGL context as `installCanvasContexts` hands it out, with what it was asked to upload. */
export type FakeGl = ReturnType<typeof createGl>
/** A Canvas 2D context as `installCanvasContexts` hands it out. */
export type Fake2d = ReturnType<typeof create2d>
export type FakeContext = FakeGl | Fake2d

export interface Gpu {
    /** The WebGL types the GPU offers (default both). */
    available?: readonly ('webgl2' | 'webgl')[]
    /** The ones whose shaders compile (default every available one). */
    compiling?: readonly ('webgl2' | 'webgl')[]
}

/**
 * Canvases as a browser hands out contexts: the first type asked for is the canvas's for
 * life, and any other type answers null. `gpu` names the WebGL types the GPU offers and
 * the ones whose shaders compile.
 */
export function installCanvasContexts(gpu: Gpu = {}) {
    const {available = ['webgl2', 'webgl'], compiling = available} = gpu
    const contexts = new WeakMap<HTMLCanvasElement, FakeContext>()
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(function (this: HTMLCanvasElement, type: string) {
        const held = contexts.get(this)
        if (held) return held.type === type ? asContext(held) : null
        let ctx: FakeContext | null = null
        if (type === '2d') ctx = create2d()
        else if (isGlType(type)) {
            const family = type === 'experimental-webgl' ? 'webgl' : type
            if (available.includes(family)) ctx = createGl(type, {compiles: compiling.includes(family)})
        }
        if (ctx) contexts.set(this, ctx)
        return ctx && asContext(ctx)
    })

    function contextOf(canvas: HTMLCanvasElement): FakeContext | null {
        return contexts.get(canvas) ?? null
    }

    return {
        contextOf,
        /** The canvas's WebGL context; throws if it holds none, or a 2D one. */
        glOf(canvas: HTMLCanvasElement): FakeGl {
            const ctx = contextOf(canvas)
            if (!ctx || ctx.type === '2d') throw new Error(`the canvas holds ${ctx ? 'a 2d' : 'no'} context`)
            return ctx
        },
        /** The canvas's Canvas 2D context; throws if it holds none, or a WebGL one. */
        context2dOf(canvas: HTMLCanvasElement): Fake2d {
            const ctx = contextOf(canvas)
            if (ctx?.type !== '2d') throw new Error(`the canvas holds ${ctx ? `a ${ctx.type}` : 'no'} context`)
            return ctx
        },
    }
}

function isGlType(type: string): type is GlContextType {
    return type === 'webgl2' || type === 'webgl' || type === 'experimental-webgl'
}

/** The one cast: a fake stands in for the platform's context. */
function asContext(ctx: FakeContext) {
    return ctx as unknown as RenderingContext
}

/** Runs `fn` with the renderers' console chatter silenced. */
export function quietly<T>(fn: () => T): T {
    const spies = (['log', 'warn', 'error'] as const).map((level) => vi.spyOn(console, level).mockImplementation(() => {}))
    try {
        return fn()
    } finally {
        spies.forEach((spy) => spy.mockRestore())
    }
}
