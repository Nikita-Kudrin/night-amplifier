import {ref} from 'vue'

/* global ImageBitmap */

// WebGL1 Shaders (GLSL ES 1.0)
const vertexShaderSourceGL1 = `
  attribute vec2 a_position;
  attribute vec2 a_texCoord;
  varying vec2 v_texCoord;

  void main() {
    gl_Position = vec4(a_position, 0.0, 1.0);
    v_texCoord = a_texCoord;
  }
`

const fragmentShaderSourceGL1 = `
  precision highp float;
  varying vec2 v_texCoord;
  uniform sampler2D u_texture;

  void main() {
    vec4 color = texture2D(u_texture, v_texCoord);
    gl_FragColor = vec4(color.rgb, 1.0);
  }
`

// WebGL2 Shaders (GLSL ES 3.0)
const vertexShaderSourceGL2 = `#version 300 es
  in vec2 a_position;
  in vec2 a_texCoord;
  out vec2 v_texCoord;

  void main() {
    gl_Position = vec4(a_position, 0.0, 1.0);
    v_texCoord = a_texCoord;
  }
`

const fragmentShaderSourceGL2 = `#version 300 es
  precision highp float;
  in vec2 v_texCoord;
  uniform sampler2D u_texture;
  out vec4 fragColor;

  void main() {
    fragColor = texture(u_texture, v_texCoord);
  }
`

function createShader(gl, type, source) {
    const shader = gl.createShader(type)
    gl.shaderSource(shader, source)
    gl.compileShader(shader)

    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
        console.error('Shader compile error:', gl.getShaderInfoLog(shader))
        gl.deleteShader(shader)
        return null
    }
    return shader
}

/** A linked program, or null; the shaders are freed either way (the program keeps them). */
function createProgram(gl, version) {
    const vertexShader = createShader(gl, gl.VERTEX_SHADER, version.vertexShader)
    const fragmentShader = createShader(gl, gl.FRAGMENT_SHADER, version.fragmentShader)
    if (!vertexShader || !fragmentShader) {
        if (vertexShader) gl.deleteShader(vertexShader)
        if (fragmentShader) gl.deleteShader(fragmentShader)
        console.error(`[WebGLRenderer] ${version.name} shader compilation failed`)
        return null
    }

    const prog = gl.createProgram()
    gl.attachShader(prog, vertexShader)
    gl.attachShader(prog, fragmentShader)
    gl.linkProgram(prog)
    gl.deleteShader(vertexShader)
    gl.deleteShader(fragmentShader)

    if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
        console.error(`[WebGLRenderer] ${version.name} program link error:`, gl.getProgramInfoLog(prog))
        gl.deleteProgram(prog)
        return null
    }
    return prog
}

const VERSIONS = [
    {
        name: 'WebGL2', backend: 'webgl2-8bit', contextTypes: ['webgl2'],
        vertexShader: vertexShaderSourceGL2, fragmentShader: fragmentShaderSourceGL2,
    },
    {
        name: 'WebGL1', backend: 'webgl1', contextTypes: ['webgl', 'experimental-webgl'],
        vertexShader: vertexShaderSourceGL1, fragmentShader: fragmentShaderSourceGL1,
    },
]

function getContext(canvas, version) {
    for (const type of version.contextTypes) {
        const ctx = canvas.getContext(type)
        if (ctx) return ctx
    }
    return null
}

/**
 * Whether `version` gets a context and builds its program, judged on a throwaway canvas.
 * A canvas hands out one context type for life, so a WebGL2 context whose shaders then
 * failed left the real canvas unable to fall back to WebGL1 or Canvas2D: it stayed blank.
 */
function versionWorks(version) {
    const ctx = getContext(document.createElement('canvas'), version)
    if (!ctx) return false
    const prog = createProgram(ctx, version)
    if (prog) ctx.deleteProgram(prog)
    // Browsers cap live contexts (~16); don't wait for the GC to free this one.
    ctx.getExtension('WEBGL_lose_context')?.loseContext()
    return prog !== null
}

/**
 * WebGL renderer composable for high-performance image rendering
 * Supports WebGL2 and WebGL1 with standard 8-bit RGB textures
 * @param {Object} [options]
 * @param {Function} [options.onRestored] - Redraws after the browser restored a lost
 *   context: the canvas is blank until then, and an idle stream sends no new frame.
 */
export function useWebGLRenderer({onRestored} = {}) {
    const backend = ref('unknown') // 'webgl2-8bit', 'webgl1', 'none'

    let gl = null
    let version = null
    let boundCanvas = null
    let program = null
    let texture = null
    let positionBuffer = null
    let texCoordBuffer = null

    function setupBuffers() {
        const positions = new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1])

        const texCoords = new Float32Array([0, 1, 1, 1, 0, 0, 1, 0])

        positionBuffer = gl.createBuffer()
        gl.bindBuffer(gl.ARRAY_BUFFER, positionBuffer)
        gl.bufferData(gl.ARRAY_BUFFER, positions, gl.STATIC_DRAW)

        texCoordBuffer = gl.createBuffer()
        gl.bindBuffer(gl.ARRAY_BUFFER, texCoordBuffer)
        gl.bufferData(gl.ARRAY_BUFFER, texCoords, gl.STATIC_DRAW)
    }

    function setupTexture() {
        texture = gl.createTexture()
        gl.bindTexture(gl.TEXTURE_2D, texture)
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE)
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE)
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR)
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR)
    }

    function setupResources() {
        program = createProgram(gl, version)
        if (!program) return false
        setupBuffers()
        setupTexture()
        return true
    }

    function forgetResources() {
        program = null
        texture = null
        positionBuffer = null
        texCoordBuffer = null
    }

    // The context's objects die with it; `preventDefault` asks the browser to restore it.
    function handleContextLost(event) {
        event.preventDefault()
        console.warn(`[WebGLRenderer] ${version.name} context lost`)
        forgetResources()
    }

    function handleContextRestored() {
        if (!setupResources()) return
        console.log(`[WebGLRenderer] ${version.name} context restored`)
        onRestored?.()
    }

    function init(canvas) {
        if (!canvas) {
            console.warn('[WebGLRenderer] Canvas element not available')
            return false
        }

        for (const candidate of VERSIONS) {
            if (!versionWorks(candidate)) {
                console.warn(`[WebGLRenderer] ${candidate.name} not available`)
                continue
            }
            const ctx = getContext(canvas, candidate)
            if (!ctx) continue
            gl = ctx
            version = candidate
            if (!setupResources()) {
                // The canvas is spoken for now; nothing else can be tried on it.
                console.error(`[WebGLRenderer] ${candidate.name} failed on the canvas after passing its probe`)
                gl = null
                break
            }
            boundCanvas = canvas
            canvas.addEventListener('webglcontextlost', handleContextLost)
            canvas.addEventListener('webglcontextrestored', handleContextRestored)
            backend.value = candidate.backend
            console.log(`[WebGLRenderer] ${candidate.name} initialization complete -`, gl.getParameter(gl.RENDERER))
            return true
        }

        backend.value = 'none'
        return false
    }

    function render(canvas, frameData, width, height) {
        if (!gl || !program || !texture || !frameData) return

        if (canvas.width !== width || canvas.height !== height) {
            canvas.width = width
            canvas.height = height
        }

        gl.bindTexture(gl.TEXTURE_2D, texture)

        if (typeof ImageBitmap !== 'undefined' && frameData instanceof ImageBitmap) {
            gl.texImage2D(
                gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, frameData
            )
        } else {
            // RGB rows are width*3 bytes, but WebGL assumes rows padded to UNPACK_ALIGNMENT
            // (default 4). For any width not divisible by 4 (e.g. 2539 px) the unpadded
            // buffer is "too short", texImage2D fails with INVALID_OPERATION and the
            // previous frame stays on screen. Set per upload: it is context state.
            gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1)
            gl.texImage2D(
                gl.TEXTURE_2D, 0, gl.RGB, width, height, 0,
                gl.RGB, gl.UNSIGNED_BYTE, frameData
            )
        }

        gl.viewport(0, 0, width, height)
        gl.clearColor(0, 0, 0, 1)
        gl.clear(gl.COLOR_BUFFER_BIT)

        gl.useProgram(program)

        const positionLoc = gl.getAttribLocation(program, 'a_position')
        gl.bindBuffer(gl.ARRAY_BUFFER, positionBuffer)
        gl.enableVertexAttribArray(positionLoc)
        gl.vertexAttribPointer(positionLoc, 2, gl.FLOAT, false, 0, 0)

        const texCoordLoc = gl.getAttribLocation(program, 'a_texCoord')
        gl.bindBuffer(gl.ARRAY_BUFFER, texCoordBuffer)
        gl.enableVertexAttribArray(texCoordLoc)
        gl.vertexAttribPointer(texCoordLoc, 2, gl.FLOAT, false, 0, 0)

        const textureLoc = gl.getUniformLocation(program, 'u_texture')
        gl.uniform1i(textureLoc, 0)

        gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4)
    }

    function cleanup() {
        if (gl) {
            if (texture) gl.deleteTexture(texture)
            if (positionBuffer) gl.deleteBuffer(positionBuffer)
            if (texCoordBuffer) gl.deleteBuffer(texCoordBuffer)
            if (program) gl.deleteProgram(program)
        }
        boundCanvas?.removeEventListener('webglcontextlost', handleContextLost)
        boundCanvas?.removeEventListener('webglcontextrestored', handleContextRestored)
        boundCanvas = null
        gl = null
        version = null
        forgetResources()
        backend.value = 'unknown'
    }

    function isInitialized() {
        return backend.value !== 'unknown' && backend.value !== 'none'
    }

    return {
        backend,
        init,
        render,
        cleanup,
        isInitialized,
    }
}
