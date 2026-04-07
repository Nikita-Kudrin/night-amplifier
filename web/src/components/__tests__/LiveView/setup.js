import {vi} from 'vitest'
import {ref, shallowRef} from 'vue'
import {mount} from '@vue/test-utils'

// Mock useImageStream
vi.mock('../../../composables/useWebSocket.js', () => ({
    useImageStream: vi.fn(),
}))

// Mock useWebGLRenderer
vi.mock('../../../composables/useWebGLRenderer.js', () => ({
    useWebGLRenderer: vi.fn(),
}))

// Mock useCanvas2DRenderer
vi.mock('../../../composables/useCanvas2DRenderer.js', () => ({
    useCanvas2DRenderer: vi.fn(),
}))

// Mock usePanZoom
vi.mock('../../../composables/usePanZoom.js', () => ({
    usePanZoom: vi.fn(),
}))

import {useImageStream} from '../../../composables/useWebSocket.js'
import {useWebGLRenderer} from '../../../composables/useWebGLRenderer.js'
import {useCanvas2DRenderer} from '../../../composables/useCanvas2DRenderer.js'
import {usePanZoom} from '../../../composables/usePanZoom.js'
import LiveView from '../../LiveView.vue'

export function createMockWebGLContext(isWebGL2 = true) {
    const ctx = {
        // WebGL constants
        VERTEX_SHADER: 35633,
        FRAGMENT_SHADER: 35632,
        ARRAY_BUFFER: 34962,
        STATIC_DRAW: 35044,
        FLOAT: 5126,
        TEXTURE_2D: 3553,
        TEXTURE_WRAP_S: 10242,
        TEXTURE_WRAP_T: 10243,
        TEXTURE_MIN_FILTER: 10241,
        TEXTURE_MAG_FILTER: 10240,
        CLAMP_TO_EDGE: 33071,
        LINEAR: 9729,
        NEAREST: 9728,
        RGB: 6407,
        UNSIGNED_BYTE: 5121,
        TRIANGLE_STRIP: 5,
        COLOR_BUFFER_BIT: 16384,
        COMPILE_STATUS: 35713,
        LINK_STATUS: 35714,
        NO_ERROR: 0,

        // WebGL2 constants for 16-bit texture support
        RGB16UI: 36214,
        RGB_INTEGER: 36248,
        UNSIGNED_SHORT: 5123,

        // Parameter constants for getParameter
        RENDERER: 7937,
        VENDOR: 7936,
        VERSION: 7938,
        SHADING_LANGUAGE_VERSION: 35724,

        // Methods
        getParameter: vi.fn((param) => {
            switch (param) {
                case 7937:
                    return 'Mock WebGL Renderer'
                case 7936:
                    return 'Mock Vendor'
                case 7938:
                    return isWebGL2 ? 'WebGL 2.0' : 'WebGL 1.0'
                case 35724:
                    return isWebGL2 ? 'WebGL GLSL ES 3.00' : 'WebGL GLSL ES 1.00'
                default:
                    return null
            }
        }),
        getError: vi.fn(() => 0), // NO_ERROR

        createShader: vi.fn(() => ({})),
        shaderSource: vi.fn(),
        compileShader: vi.fn(),
        getShaderParameter: vi.fn(() => true),
        getShaderInfoLog: vi.fn(() => ''),
        deleteShader: vi.fn(),

        createProgram: vi.fn(() => ({})),
        attachShader: vi.fn(),
        linkProgram: vi.fn(),
        getProgramParameter: vi.fn(() => true),
        getProgramInfoLog: vi.fn(() => ''),
        deleteProgram: vi.fn(),
        useProgram: vi.fn(),

        createBuffer: vi.fn(() => ({})),
        bindBuffer: vi.fn(),
        bufferData: vi.fn(),
        deleteBuffer: vi.fn(),

        createTexture: vi.fn(() => ({})),
        bindTexture: vi.fn(),
        texParameteri: vi.fn(),
        texImage2D: vi.fn(),
        deleteTexture: vi.fn(),

        getAttribLocation: vi.fn(() => 0),
        enableVertexAttribArray: vi.fn(),
        vertexAttribPointer: vi.fn(),

        getUniformLocation: vi.fn(() => ({})),
        uniform1i: vi.fn(),

        viewport: vi.fn(),
        clearColor: vi.fn(),
        clear: vi.fn(),
        drawArrays: vi.fn(),
    }

    return ctx
}

export function createMockImageStream() {
    return {
        connected: ref(true),
        frameData: shallowRef(null),
        dimensions: ref({width: 0, height: 0}),
        frameNumber: ref(0),
        decodeError: ref(null),
        connect: vi.fn(),
        disconnect: vi.fn(),
    }
}

export function createMockWebGLRenderer() {
    return {
        backend: ref('webgl2-10bit'),
        init: vi.fn(() => true),
        render: vi.fn(),
        cleanup: vi.fn(),
        isInitialized: vi.fn(() => true),
    }
}

export function createMockCanvas2DRenderer() {
    return {
        backend: ref('canvas2d'),
        init: vi.fn(() => false),
        render: vi.fn(),
        cleanup: vi.fn(),
        isInitialized: vi.fn(() => false),
    }
}

export function createMockPanZoom() {
    const isDragging = ref(false)
    const scaleRef = ref(1)
    const positionRef = ref({x: 0, y: 0})
    const isFullscreenRef = ref(false)

    return {
        scale: scaleRef,
        position: positionRef,
        isDragging: isDragging,
        isFullscreen: isFullscreenRef,
        canvasStyle: {
            get transform() {
                return `translate(${positionRef.value.x}px, ${positionRef.value.y}px) scale(${scaleRef.value})`
            },
            get cursor() {
                return isDragging.value ? 'grabbing' : 'grab'
            },
        },
        handleWheel: vi.fn((e) => {
            e.preventDefault?.()
            scaleRef.value = scaleRef.value * 1.1
        }),
        handleMouseDown: vi.fn((e) => {
            if (e.button === 0) isDragging.value = true
        }),
        handleMouseMove: vi.fn(),
        handleMouseUp: vi.fn(() => {
            isDragging.value = false
        }),
        handleTouchStart: vi.fn((e) => {
            if (e.touches?.length === 1) isDragging.value = true
        }),
        handleTouchMove: vi.fn((e) => {
            if (e.touches?.length === 2) {
                scaleRef.value = 1.5
            }
        }),
        handleTouchEnd: vi.fn(() => {
            isDragging.value = false
        }),
        zoomIn: vi.fn(),
        zoomOut: vi.fn(),
        resetView: vi.fn(() => {
            scaleRef.value = 1
            positionRef.value = {x: 0, y: 0}
        }),
        fitToView: vi.fn(),
        toggleFullscreen: vi.fn(() => {
            Element.prototype.requestFullscreen()
        }),
        handleFullscreenChange: vi.fn(),
    }
}

export function createMockProvides(overrides = {}) {
    return {
        eventStream: {
            captureState: ref(overrides.captureState ?? 'Idle'),
            pushDirection: ref(overrides.pushDirection ?? null),
            currentTarget: ref(overrides.currentTarget ?? null),
            plateSolving: ref(overrides.plateSolving ?? {inProgress: false, targetName: null, lastResult: null}),
            ...overrides.eventStream,
        },
        settings: ref({
            stacking_type: 'deep_sky',
            comet_roi: null,
            ...overrides.settings,
        }),
    }
}

export function createMockFrameData(width, height, fillValue = 100) {
    const data = new Uint8Array(width * height * 4)
    for (let i = 0; i < width * height; i++) {
        const offset = i * 4
        const r = fillValue & 0x3ff
        const g = fillValue & 0x3ff
        const b = fillValue & 0x3ff
        const a = 3
        const packed = (a << 30) | (b << 20) | (g << 10) | r
        data[offset] = packed & 0xff
        data[offset + 1] = (packed >> 8) & 0xff
        data[offset + 2] = (packed >> 16) & 0xff
        data[offset + 3] = (packed >> 24) & 0xff
    }
    return data
}

export function mountLiveView(provides = {}) {
    return mount(LiveView, {
        global: {
            provide: createMockProvides(provides),
        },
    })
}

export function setupMocks() {
    const mockImageStream = createMockImageStream()
    const mockWebGLRenderer = createMockWebGLRenderer()
    const mockCanvas2DRenderer = createMockCanvas2DRenderer()
    const mockPanZoom = createMockPanZoom()
    const mockWebGLContext = createMockWebGLContext()

    useImageStream.mockReturnValue(mockImageStream)
    useWebGLRenderer.mockReturnValue(mockWebGLRenderer)
    useCanvas2DRenderer.mockReturnValue(mockCanvas2DRenderer)
    usePanZoom.mockReturnValue(mockPanZoom)

    HTMLCanvasElement.prototype.getContext = vi.fn((contextType) => {
        if (contextType === 'webgl2' || contextType === 'webgl') {
            return mockWebGLContext
        }
        return null
    })

    Element.prototype.requestFullscreen = vi.fn().mockResolvedValue(undefined)
    document.exitFullscreen = vi.fn().mockResolvedValue(undefined)

    return {
        mockImageStream,
        mockWebGLRenderer,
        mockCanvas2DRenderer,
        mockPanZoom,
        mockWebGLContext,
    }
}

export {useImageStream, useWebGLRenderer, useCanvas2DRenderer, usePanZoom}
