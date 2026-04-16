import {ref} from 'vue'

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

function createProgram(glCtx, vertexShader, fragmentShader) {
    const prog = glCtx.createProgram()
    glCtx.attachShader(prog, vertexShader)
    glCtx.attachShader(prog, fragmentShader)
    glCtx.linkProgram(prog)

    if (!glCtx.getProgramParameter(prog, glCtx.LINK_STATUS)) {
        console.error('Program link error:', glCtx.getProgramInfoLog(prog))
        glCtx.deleteProgram(prog)
        return null
    }
    return prog
}

/**
 * WebGL renderer composable for high-performance image rendering
 * Supports WebGL2 and WebGL1 with standard 8-bit RGB textures
 */
export function useWebGLRenderer() {
    const backend = ref('unknown') // 'webgl2-8bit', 'webgl1', 'none'

    let gl = null
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

    function initWebGL2(canvas) {
        console.log('[WebGLRenderer] Attempting WebGL2 initialization...')

        const gl2 = canvas.getContext('webgl2')
        if (!gl2) {
            console.warn('[WebGLRenderer] WebGL2 not available')
            return false
        }

        console.log('[WebGLRenderer] WebGL2 context created')
        console.log('[WebGLRenderer] Renderer:', gl2.getParameter(gl2.RENDERER))

        gl = gl2

        const vertexShader = createShader(gl, gl.VERTEX_SHADER, vertexShaderSourceGL2)
        const fragmentShader = createShader(gl, gl.FRAGMENT_SHADER, fragmentShaderSourceGL2)
        program = createProgram(gl, vertexShader, fragmentShader)

        if (!program) {
            console.error('[WebGLRenderer] WebGL2 shader compilation/linking failed')
            gl = null
            return false
        }

        setupBuffers()
        setupTexture()

        backend.value = 'webgl2-8bit'
        console.log('[WebGLRenderer] WebGL2 initialization complete -', backend.value)
        return true
    }

    function initWebGL1(canvas) {
        console.log('[WebGLRenderer] Attempting WebGL1 initialization...')

        const gl1 = canvas.getContext('webgl') || canvas.getContext('experimental-webgl')
        if (!gl1) {
            console.warn('[WebGLRenderer] WebGL1 not available')
            return false
        }

        console.log('[WebGLRenderer] WebGL1 context created')

        gl = gl1

        const vertexShader = createShader(gl, gl.VERTEX_SHADER, vertexShaderSourceGL1)
        const fragmentShader = createShader(gl, gl.FRAGMENT_SHADER, fragmentShaderSourceGL1)

        if (!vertexShader || !fragmentShader) {
            console.error('[WebGLRenderer] WebGL1 shader compilation failed')
            gl = null
            return false
        }

        program = createProgram(gl, vertexShader, fragmentShader)
        if (!program) {
            console.error('[WebGLRenderer] WebGL1 program linking failed')
            gl = null
            return false
        }

        setupBuffers()
        setupTexture()

        backend.value = 'webgl1'
        console.log('[WebGLRenderer] WebGL1 initialization complete')
        return true
    }

    function init(canvas) {
        if (!canvas) {
            console.warn('[WebGLRenderer] Canvas element not available')
            return false
        }

        if (initWebGL2(canvas)) {
            return true
        }

        if (initWebGL1(canvas)) {
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

        // Standard 8-bit texture upload — works identically on WebGL1 and WebGL2
        gl.texImage2D(
            gl.TEXTURE_2D, 0, gl.RGB, width, height, 0,
            gl.RGB, gl.UNSIGNED_BYTE, frameData
        )

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
        gl = null
        program = null
        texture = null
        positionBuffer = null
        texCoordBuffer = null
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
