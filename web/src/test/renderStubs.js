import {vi} from 'vitest'

/**
 * Canvas contexts as a browser hands them out, for renderer tests: real enum names, a
 * `texImage2D` that validates row padding the way the spec does, and one context type per
 * canvas for life. Restored by `vi.restoreAllMocks()`.
 */

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
export function installCanvasContexts(gpu = {}) {
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

/** Runs `fn` with the renderers' console chatter silenced. */
export function quietly(fn) {
    const spies = ['log', 'warn', 'error'].map((level) => vi.spyOn(console, level).mockImplementation(() => {}))
    try {
        return fn()
    } finally {
        spies.forEach((spy) => spy.mockRestore())
    }
}
