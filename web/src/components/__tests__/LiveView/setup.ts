import {computed, ref, shallowRef} from 'vue'
import {mount} from '@vue/test-utils'

// Mock useImageStream
vi.mock('../../../composables/useImageStream.js', () => ({
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

import {useImageStream} from '../../../composables/useImageStream.js'
import type {FrameData, FrameSize, ImageStream} from '../../../composables/useImageStream.js'
import {useWebGLRenderer} from '../../../composables/useWebGLRenderer.js'
import type {RendererBackend, WebGLRenderer} from '../../../composables/useWebGLRenderer.js'
import {useCanvas2DRenderer} from '../../../composables/useCanvas2DRenderer.js'
import type {Canvas2DRenderer} from '../../../composables/useCanvas2DRenderer.js'
import {usePanZoom} from '../../../composables/usePanZoom.js'
import type {View} from '../../../composables/usePanZoom.js'
import {useFullscreen} from '../../../composables/useFullscreen.js'
import LiveView from '../../LiveView.vue'
import {contextProvides} from '../../../composables/appContext.js'
import type {CaptureState} from '../../../composables/api.events.js'
import type {Camera, CameraRole, Settings} from '../../../composables/api.types.js'
import type {ClientPushDirection, CurrentTarget} from '../../../composables/events/pushToEvents.js'
import {createEventStreamStub} from '../../../test/eventStream.js'
import {installCanvasContexts} from '../../../test/renderStubs.js'
import {settings} from '../../../test/fixtures.js'

type PanZoom = ReturnType<typeof usePanZoom>

/**
 * Options LiveView handed `useImageStream` on its most recent mount.
 *
 * A function rather than a re-export: the endpoint is a getter now, and reading it
 * through the live mock is the only way to observe which source the view is on.
 */
export function lastImageStreamOptions() {
    return vi.mocked(useImageStream).mock.lastCall?.[0] ?? null
}

/** A push direction as the event stream holds it; the fields a test names, the rest neutral. */
export function createMockImageStream() {
    return {
        connected: ref(true),
        error: ref<string | null>(null),
        frameData: shallowRef<FrameData | null>(null),
        dimensions: ref<FrameSize>({width: 0, height: 0}),
        frameNumber: ref(0),
        fps: ref(0),
        decodeError: ref<string | null>(null),
        isJpeg: ref(false),
        isDynamicJpeg: false,
        connect: vi.fn(),
        disconnect: vi.fn(),
        clearFrameData: vi.fn(),
    } satisfies ImageStream
}

export function createMockWebGLRenderer() {
    return {
        backend: ref<RendererBackend>('webgl2-8bit'),
        init: vi.fn<WebGLRenderer['init']>(() => true),
        render: vi.fn<WebGLRenderer['render']>(() => true), // drew, as a live context does
        cleanup: vi.fn(),
        isInitialized: vi.fn(() => true),
    } satisfies WebGLRenderer
}

export function createMockCanvas2DRenderer() {
    return {
        backend: ref<RendererBackend>('canvas2d'),
        init: vi.fn<Canvas2DRenderer['init']>(() => false),
        render: vi.fn<Canvas2DRenderer['render']>(() => true),
        cleanup: vi.fn(),
        isInitialized: vi.fn(() => false),
    } satisfies Canvas2DRenderer
}

export function createMockPanZoom() {
    const isDragging = ref(false)
    const scaleRef = ref(1)
    const positionRef = ref<View['position']>({x: 0, y: 0})
    const isFullscreenRef = ref(false)

    return {
        scale: scaleRef,
        position: positionRef,
        isDragging: isDragging,
        isFullscreen: isFullscreenRef,
        canvasStyle: computed(() => ({
            transform: `translate(${positionRef.value.x}px, ${positionRef.value.y}px) scale(${scaleRef.value})`,
            cursor: isDragging.value ? 'grabbing' : 'grab',
        })),
        handleWheel: vi.fn<PanZoom['handleWheel']>((e) => {
            e.preventDefault?.()
            scaleRef.value = scaleRef.value * 1.1
        }),
        handleMouseDown: vi.fn<PanZoom['handleMouseDown']>((e) => {
            if (e.button === 0) isDragging.value = true
        }),
        handleMouseMove: vi.fn<PanZoom['handleMouseMove']>(),
        handleMouseUp: vi.fn(() => {
            isDragging.value = false
        }),
        handleTouchStart: vi.fn<PanZoom['handleTouchStart']>((e) => {
            if (e.touches?.length === 1) isDragging.value = true
        }),
        handleTouchMove: vi.fn<PanZoom['handleTouchMove']>((e) => {
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
        fitToView: vi.fn<PanZoom['fitToView']>(),
        toggleFullscreen: vi.fn<PanZoom['toggleFullscreen']>(() => {
            Element.prototype.requestFullscreen()
        }),
        handleFullscreenChange: vi.fn(),
    } satisfies PanZoom
}

export type MockPanZoom = ReturnType<typeof createMockPanZoom>

/**
 * Swaps the mock's fullscreen half for the real `useFullscreen`, so a dispatched
 * `fullscreenchange` goes through the actual edge detection into LiveView's `onChange`.
 */
export function withRealFullscreen(mockPanZoom: MockPanZoom) {
    vi.mocked(usePanZoom).mockImplementation((options) => ({...mockPanZoom, ...useFullscreen(options)}))
}

export function setFullscreenElement(element: Element | null) {
    Object.defineProperty(document, 'fullscreenElement', {
        value: element,
        writable: true,
        configurable: true,
    })
}

export function changeFullscreen(element: Element | null) {
    setFullscreenElement(element)
    document.dispatchEvent(new Event('fullscreenchange'))
}

export {installResizeObserverStub} from '../../../test/dom.js'

/** happy-dom has no `screen.orientation`; install before mounting. */
export function installScreenOrientationStub() {
    const orientation = new window.EventTarget()
    Object.defineProperty(window.screen, 'orientation', {value: orientation, configurable: true})
    return orientation
}

export function removeScreenOrientationStub() {
    Reflect.deleteProperty(window.screen, 'orientation')
}

/** What a test sets on the view's context: the stream's state by name, and the cameras. */
export interface MockProvides {
    captureState?: CaptureState
    pushDirection?: ClientPushDirection | null
    currentTarget?: CurrentTarget | null
    viewedCamera?: CameraRole
    /** More of the stream, a ref to share with the test or a value. */
    eventStream?: Parameters<typeof createEventStreamStub>[0]
    settings?: Partial<Settings>
    hasGuideCamera?: boolean
    guideCamera?: Camera | null
    mainCamera?: Camera | null
}

export function createMockProvides(overrides: MockProvides = {}) {
    return {
        eventStream: createEventStreamStub({
            captureState: overrides.captureState ?? 'Idle',
            pushDirection: overrides.pushDirection ?? null,
            currentTarget: overrides.currentTarget ?? null,
            viewedCamera: overrides.viewedCamera ?? 'main',
            ...overrides.eventStream,
        }),
        settings: ref(settings(overrides.settings)),
        hasGuideCamera: ref(overrides.hasGuideCamera ?? false),
        guideCamera: ref(overrides.guideCamera ?? null),
        mainCamera: ref(overrides.mainCamera ?? null),
    }
}

export function createMockFrameData(width: number, height: number, fillValue = 100) {
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

export function mountLiveView(provides: MockProvides = {}) {
    return mount(LiveView, {
        global: {
            provide: contextProvides(createMockProvides(provides)),
        },
    })
}

export function setupMocks() {
    const mockImageStream = createMockImageStream()
    const mockWebGLRenderer = createMockWebGLRenderer()
    const mockCanvas2DRenderer = createMockCanvas2DRenderer()
    const mockPanZoom = createMockPanZoom()

    vi.mocked(useImageStream).mockReturnValue(mockImageStream)
    vi.mocked(useWebGLRenderer).mockReturnValue(mockWebGLRenderer)
    vi.mocked(useCanvas2DRenderer).mockReturnValue(mockCanvas2DRenderer)
    vi.mocked(usePanZoom).mockReturnValue(mockPanZoom)

    // Spies, so `vi.restoreAllMocks()` gives happy-dom its own `getContext` back.
    installCanvasContexts()

    Element.prototype.requestFullscreen = vi.fn().mockResolvedValue(undefined)
    document.exitFullscreen = vi.fn().mockResolvedValue(undefined)

    return {
        mockImageStream,
        mockWebGLRenderer,
        mockCanvas2DRenderer,
        mockPanZoom,
    }
}

export type Mocks = ReturnType<typeof setupMocks>

export {useImageStream, useWebGLRenderer, useCanvas2DRenderer, usePanZoom}
