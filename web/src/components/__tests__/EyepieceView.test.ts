import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { installResizeObserverStub } from '../../test/dom'
import { mount } from '@vue/test-utils'
import type { VueWrapper } from '@vue/test-utils'
import { nextTick, ref, shallowRef } from 'vue'
import type { Component } from 'vue'
import type { FrameData, FrameSize, ImageStream } from '../../composables/useImageStream'
import type { RendererBackend, WebGLRenderer } from '../../composables/useWebGLRenderer'
import type { Canvas2DRenderer } from '../../composables/useCanvas2DRenderer'
import type { CameraRole, EyepieceSettings, Settings } from '../../composables/api.types'
import type { EyepieceSnapshot } from '../../composables/api'
import type { AppContext } from '../../composables/appContext'

// Mock the image stream composable
vi.mock('../../composables/useImageStream', () => ({
  useImageStream: vi.fn(() => ({
    connected: ref(true),
    error: ref<string | null>(null),
    decodeError: ref<string | null>(null),
    frameData: shallowRef<FrameData | null>(null),
    dimensions: ref<FrameSize>({ width: 0, height: 0 }),
    isJpeg: ref(false),
    isDynamicJpeg: false,
    frameNumber: ref(0),
    fps: ref(0),
    connect: vi.fn(),
    disconnect: vi.fn(),
    clearFrameData: vi.fn(),
  } satisfies ImageStream))
}))

// Mock renderer composables
vi.mock('../../composables/useWebGLRenderer', () => ({
  useWebGLRenderer: vi.fn(() => ({
    init: vi.fn(() => true),
    render: vi.fn(() => true), // drew, as a live context does
    maxSourceEdge: vi.fn(() => Infinity),
    cleanup: vi.fn(),
    isInitialized: vi.fn(() => true),
    backend: ref<RendererBackend>('webgl2-8bit'),
  } satisfies WebGLRenderer))
}))

vi.mock('../../composables/useCanvas2DRenderer', () => ({
  useCanvas2DRenderer: vi.fn(() => ({
    init: vi.fn(() => false),
    render: vi.fn(() => true),
    maxSourceEdge: vi.fn(() => Infinity),
    cleanup: vi.fn(),
    isInitialized: vi.fn(() => false),
    backend: ref<RendererBackend>('canvas2d'),
  } satisfies Canvas2DRenderer))
}))

const mockSnapshotFilename = 'eyepiece_05-09-2026_21-14-03.png'
vi.mock('../../composables/api', async (importOriginal) =>
  (await import('../../test/mockApi')).mockApi(await importOriginal(), {
    fetchEyepieceSnapshot: vi.fn(async (_circular: boolean) => ({
      blob: new Blob(['png'], { type: 'image/png' }),
      filename: mockSnapshotFilename,
    })),
  }))

// Tests switch to fake timers inside their bodies; a failed assertion must not leave
// them on for the next test.
afterEach(() => {
  vi.useRealTimers()
})

import { useImageStream } from '../../composables/useImageStream'
import * as api from '../../composables/api'
import { IDLE_HIDE_MS } from '../../composables/useOverlayVisibility'
import {contextProvides} from '../../composables/appContext'
import { createEventStreamStub } from '../../test/eventStream'
import { camera, capabilities, catalogEntry, clientDirection, settings } from '../../test/fixtures'
import { FAILED_SOLVE_HOLD_MS, SOLVING_HOLD_MS } from '../../constants'
import { NOT_SOLVING } from '../../composables/events/pushToEvents'
import type { PlateSolving } from '../../composables/events/pushToEvents'

const fetchEyepieceSnapshot = vi.mocked(api.fetchEyepieceSnapshot)

/** The backend readout is debug-only, so its layout can only be tested with it on. */
const mockCapabilities = ref(capabilities({ debug_logging: false }))

/** Puts the page at `pathname`, as a route loads it; `unstubGlobals` puts it back. */
function setPath(pathname: string) {
  vi.stubGlobal('location', { ...window.location, pathname })
}

/** Sets one of the fullscreen API's read-only `document` properties. */
function setDocument(name: 'fullscreenElement' | 'fullscreenEnabled', value: Element | boolean | null) {
  Object.defineProperty(document, name, { value, writable: true, configurable: true })
}

/** The settings with `eyepiece`'s fields over the default eyepiece group. */
function eyepieceSettings(eyepiece: Partial<EyepieceSettings>, overrides: Partial<Settings> = {}) {
  return ref(settings({ eyepiece: { ...settings().eyepiece, ...eyepiece }, ...overrides }))
}

/** A push direction as the event stream holds it; the fields a test names, the rest neutral. */
/** A snapshot as the server answers it. */
function snapshot(): EyepieceSnapshot {
  return { blob: new Blob(['png'], { type: 'image/png' }), filename: mockSnapshotFilename }
}

interface MountOverrides {
  settings?: Partial<Settings>
  eventStream?: Parameters<typeof createEventStreamStub>[0]
  provide?: Partial<AppContext>
}

function mountEyepiece(eyepiece: Partial<EyepieceSettings> = { binoview: true, circular_view: true }, overrides: MountOverrides = {}) {
  return mount(EyepieceViewModule, {
    global: {
      provide: contextProvides({
        settings: eyepieceSettings(eyepiece, overrides.settings),
        capabilities: mockCapabilities,
        eventStream: createEventStreamStub(overrides.eventStream),
        ...overrides.provide,
      }),
    },
  })
}

/** The stream the most recently mounted view made. */
function lastStream(): ImageStream {
  const result = vi.mocked(useImageStream).mock.results.at(-1)
  if (result?.type !== 'return') throw new Error('no view made an image stream')
  return result.value
}

/**
 * Land a frame on the most recently mounted view. Zoom, the download buttons and
 * the placeholder all gate on having one.
 */
async function landFrame() {
  const stream = lastStream()
  stream.dimensions.value = { width: 100, height: 100 }
  stream.frameData.value = new Uint8Array(4)
  await nextTick()
}

/** Assigned by each `beforeEach` that needs a component; see `loadEyepieceView`. */
let EyepieceViewModule: Component

async function loadEyepieceView() {
  EyepieceViewModule = (await import('../EyepieceView.vue')).default
  return EyepieceViewModule
}

describe('EyepieceView.vue Endpoint Selection', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  afterEach(() => {
    vi.resetModules()
  })

  it('uses /ws/eyepiece when path is /eyepiece (JPEG streaming)', async () => {
    setPath('/eyepiece')
    
    // We import dynamically to ensure it runs after window.location is set
    const EyepieceView = (await import('../EyepieceView.vue')).default
    mount(EyepieceView, {
      global: {
        provide: contextProvides({
          settings: eyepieceSettings({ binoview: true, circular_view: true }),
          eventStream: createEventStreamStub(),
        })
      }
    })

    expect(useImageStream).toHaveBeenCalledWith({endpoint: '/ws/eyepiece'})
  })

  it('uses /ws/eyepiece_quality when path is /eyepiece_quality (raw 8bit + LZ4)', async () => {
    setPath('/eyepiece_quality')
    
    const EyepieceView = (await import('../EyepieceView.vue')).default
    mount(EyepieceView, {
      global: {
        provide: contextProvides({
          settings: eyepieceSettings({ binoview: true, circular_view: true }),
          eventStream: createEventStreamStub(),
        })
      }
    })

    // Only the endpoint: the server sizes the frame from Eyepiece Streaming Resolution.
    expect(useImageStream).toHaveBeenCalledWith({endpoint: '/ws/eyepiece_quality'})
  })

  // The size is a server setting now, so no layout change may produce a report — an
  // old per-client report would be ignored anyway, but it must not be sent at all.
  it('reports no viewport on mount, resize, rotation or binoview layout', async () => {
    vi.useFakeTimers()
    setPath('/eyepiece_quality')
    vi.stubGlobal('innerWidth', 1440)
    vi.stubGlobal('innerHeight', 1440)
    vi.stubGlobal('devicePixelRatio', 2)

    const EyepieceView = (await import('../EyepieceView.vue')).default
    const wrapper = mount(EyepieceView, {
      global: {
        provide: contextProvides({
          settings: eyepieceSettings({ binoview: true, circular_view: true }),
          eventStream: createEventStreamStub(),
        })
      }
    })

    const stream = lastStream()
    expect(stream).not.toHaveProperty('sendResolution')
    vi.stubGlobal('innerWidth', 2880)
    window.dispatchEvent(new Event('resize'))
    window.dispatchEvent(new Event('orientationchange'))
    vi.advanceTimersByTime(250)

    expect(useImageStream).toHaveBeenCalledTimes(1)
    expect(useImageStream).toHaveBeenCalledWith({endpoint: '/ws/eyepiece_quality'})
    wrapper.unmount()
    vi.useRealTimers()
  })
})

describe('EyepieceView.vue Layout', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    setPath('/eyepiece')
  })

  afterEach(() => {
    vi.resetModules()
  })

  it('applies circular class to canvas when circular_view setting is true', async () => {
    const EyepieceView = (await import('../EyepieceView.vue')).default
    const wrapper = mount(EyepieceView, {
      global: {
        provide: contextProvides({
          settings: eyepieceSettings({ binoview: false, circular_view: true }),
          eventStream: createEventStreamStub(),
        })
      }
    })

    const singleCanvas = wrapper.find('.single-view canvas')
    expect(singleCanvas.exists()).toBe(true)
    expect(singleCanvas.classes()).toContain('circular')
  })

  it('does not apply circular class to canvas when circular_view setting is false', async () => {
    const EyepieceView = (await import('../EyepieceView.vue')).default
    const wrapper = mount(EyepieceView, {
      global: {
        provide: contextProvides({
          settings: eyepieceSettings({ binoview: false, circular_view: false }),
          eventStream: createEventStreamStub(),
        })
      }
    })

    const singleCanvas = wrapper.find('.single-view canvas')
    expect(singleCanvas.exists()).toBe(true)
    expect(singleCanvas.classes()).not.toContain('circular')
  })

  it('renders GuideArrow when pushDirection and currentTarget are present', async () => {
    installResizeObserverStub()

    const EyepieceView = (await import('../EyepieceView.vue')).default
    const wrapper = mount(EyepieceView, {
      global: {
        provide: contextProvides({
          settings: eyepieceSettings({ binoview: true, circular_view: true }),
          eventStream: createEventStreamStub({
            pushDirection: clientDirection({ angleDeg: 45, distanceDeg: 10, isClose: false, directionHint: 'NE' }),
            currentTarget: catalogEntry(),
          }),
        })
      }
    })

    // `/eyepiece` is monocular: one eye, one arrow, and no hidden binoview pair beside it.
    const arrows = wrapper.findAllComponents({ name: 'GuideArrow' })
    expect(arrows.length).toBe(1)
  })

  it('does not render GuideArrow when pushDirection is null and nothing is solving', async () => {
    installResizeObserverStub()

    const EyepieceView = (await import('../EyepieceView.vue')).default
    const wrapper = mount(EyepieceView, {
      global: {
        provide: contextProvides({
          settings: eyepieceSettings({ binoview: false, circular_view: true }),
          eventStream: createEventStreamStub({
            currentTarget: catalogEntry(),
          }),
        })
      }
    })

    const arrows = wrapper.findAllComponents({ name: 'GuideArrow' })
    expect(arrows.length).toBe(0)
  })
})

/**
 * `/eyepiece` is the view an observer puts their eye to, so it is monocular
 * whatever Binoview says. `/eyepiece_quality` is the diagnostic view and still
 * honours the setting.
 */
describe('EyepieceView.vue Monocular Route', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    installResizeObserverStub()
  })

  afterEach(() => {
    vi.resetModules()
  })

  /** Only the eyes of the layout on screen are mounted. */
  function isHidden(wrapper: VueWrapper, selector: string) {
    return !wrapper.find(selector).exists()
  }

  it('stays monocular on /eyepiece even with binoview enabled', async () => {
    setPath('/eyepiece')
    await loadEyepieceView()

    const wrapper = mountEyepiece({ binoview: true, circular_view: true })

    expect(isHidden(wrapper, '.binoview-container')).toBe(true)
    expect(isHidden(wrapper, '.single-view')).toBe(false)
  })

  it('honours binoview on /eyepiece_quality', async () => {
    setPath('/eyepiece_quality')
    await loadEyepieceView()

    const wrapper = mountEyepiece({ binoview: true, circular_view: true })

    expect(isHidden(wrapper, '.binoview-container')).toBe(false)
    expect(isHidden(wrapper, '.single-view')).toBe(true)
  })

  it('is monocular on /eyepiece_quality when binoview is off', async () => {
    setPath('/eyepiece_quality')
    await loadEyepieceView()

    const wrapper = mountEyepiece({ binoview: false, circular_view: true })

    expect(isHidden(wrapper, '.binoview-container')).toBe(true)
    expect(isHidden(wrapper, '.single-view')).toBe(false)
  })
})

describe('EyepieceView.vue Fullscreen', () => {
  beforeEach(async () => {
    vi.clearAllMocks()
    setPath('/eyepiece')
    installResizeObserverStub()
    Element.prototype.requestFullscreen = vi.fn().mockResolvedValue(undefined)
    document.exitFullscreen = vi.fn().mockResolvedValue(undefined)
    setDocument('fullscreenElement', null)
    setDocument('fullscreenEnabled', true)
    await loadEyepieceView()
  })

  afterEach(() => {
    vi.resetModules()
  })

  function fullscreenButton(wrapper: VueWrapper) {
    return wrapper.findAll('.eyepiece-btn').find((b) => b.attributes('title')?.includes('ullscreen'))
  }

  it('renders a fullscreen button', () => {
    const wrapper = mountEyepiece()

    expect(fullscreenButton(wrapper)).toBeTruthy()
  })

  // iPhone Safari implements no Fullscreen API, so the button would be a control
  // that visibly does nothing on a phone at the eyepiece.
  it('hides the button where the browser has no fullscreen', async () => {
    setDocument('fullscreenEnabled', false)
    try {
      const wrapper = mountEyepiece()
      expect(fullscreenButton(wrapper)).toBeUndefined()
    } finally {
      setDocument('fullscreenEnabled', true)
    }
  })

  it('requests fullscreen when clicked', async () => {
    const wrapper = mountEyepiece()

    await fullscreenButton(wrapper)?.trigger('click')

    expect(Element.prototype.requestFullscreen).toHaveBeenCalled()
  })

  // Entering fullscreen fits all, which for this view means dropping back to an
  // unzoomed, unpanned frame.
  it('fits all when the browser confirms fullscreen', async () => {
    const wrapper = mountEyepiece()
    await landFrame()
    // Zoom in first, so a reset is observable.
    await wrapper.trigger('wheel', {deltaY: -2000, clientX: 100, clientY: 100})
    expect(wrapper.find('.fit-all-btn').exists()).toBe(true)

    setDocument('fullscreenElement', document.body)
    document.dispatchEvent(new Event('fullscreenchange'))
    await nextTick()

    expect(wrapper.find('.fit-all-btn').exists()).toBe(false)
  })

  it('fits all on a rotation while fullscreen', async () => {
    vi.useFakeTimers()
    const wrapper = mountEyepiece()
    await landFrame()
    setDocument('fullscreenElement', document.body)
    document.dispatchEvent(new Event('fullscreenchange'))
    await wrapper.trigger('wheel', {deltaY: -2000, clientX: 100, clientY: 100})
    expect(wrapper.find('.fit-all-btn').exists()).toBe(true)

    window.dispatchEvent(new Event('orientationchange'))
    vi.advanceTimersByTime(250)
    await nextTick()

    expect(wrapper.find('.fit-all-btn').exists()).toBe(false)
    vi.useRealTimers()
  })

  // Rotation fires no `orientationchange` on some browsers, only `screen.orientation`'s.
  it('fits all on a screen orientation change while fullscreen', async () => {
    const orientation = new window.EventTarget()
    Object.defineProperty(window.screen, 'orientation', { value: orientation, configurable: true })
    try {
      const wrapper = mountEyepiece()
      await landFrame()
      setDocument('fullscreenElement', document.body)
      document.dispatchEvent(new Event('fullscreenchange'))
      await wrapper.trigger('wheel', {deltaY: -2000, clientX: 100, clientY: 100})
      expect(wrapper.find('.fit-all-btn').exists()).toBe(true)

      orientation.dispatchEvent(new Event('change'))
      await nextTick()

      expect(wrapper.find('.fit-all-btn').exists()).toBe(false)
    } finally {
      Reflect.deleteProperty(window.screen, 'orientation')
    }
  })

  // Windowed, the zoom is the user's own and a resize must not throw it away.
  it('leaves the zoom alone on a resize outside fullscreen', async () => {
    vi.useFakeTimers()
    const wrapper = mountEyepiece()
    await landFrame()
    await wrapper.trigger('wheel', {deltaY: -2000, clientX: 100, clientY: 100})

    window.dispatchEvent(new Event('resize'))
    vi.advanceTimersByTime(250)
    await nextTick()

    expect(wrapper.find('.fit-all-btn').exists()).toBe(true)
    vi.useRealTimers()
  })
})

describe('EyepieceView.vue Download', () => {
  beforeEach(async () => {
    vi.clearAllMocks()
    setPath('/eyepiece')
    installResizeObserverStub()
    vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:snapshot')
    vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {})
    await loadEyepieceView()
  })

  afterEach(() => {
    vi.resetModules()
  })

  function splitButton(wrapper: VueWrapper) {
    return wrapper.findComponent({ name: 'BaseSplitButton' })
  }

  it('shows an icon rather than the word Download', () => {
    const wrapper = mountEyepiece()

    const main = wrapper.find('.download-btn .split-button-main')
    expect(main.exists()).toBe(true)
    expect(main.text()).toBe('')
    expect(main.find('svg').exists()).toBe(true)
  })

  // Icon-only, so the label has to survive as a tooltip and an accessible name.
  it('names the icon button in its tooltip', () => {
    const wrapper = mountEyepiece()

    const main = wrapper.find('.download-btn .split-button-main')
    expect(main.attributes('title')).toBe('Download')
    expect(main.attributes('aria-label')).toBe('Download')
  })

  it('offers Download original in the menu', () => {
    const wrapper = mountEyepiece()

    expect(splitButton(wrapper).props('options')[0].label).toBe('Download original')
  })

  it('downloads the round image from the primary action', async () => {
    const wrapper = mountEyepiece()

    await splitButton(wrapper).vm.$emit('click')
    await nextTick()

    expect(fetchEyepieceSnapshot).toHaveBeenCalledWith(true)
  })

  it('downloads the uncropped image from the menu', async () => {
    const wrapper = mountEyepiece()

    await splitButton(wrapper).vm.$emit('select', 'original')
    await nextTick()

    expect(fetchEyepieceSnapshot).toHaveBeenCalledWith(false)
  })

  /**
   * The bytes must land on the device, under the name the server stamped a
   * timestamp into. A `blob:` URL carries no `Content-Disposition` — `fetch` has
   * consumed it — so the attribute is both what names the file and what makes this
   * a save at all: without it the browser navigates to the blob and renders the PNG
   * in the tab, taking the eyepiece stream down with the page.
   */
  it('saves the file under the name the server gave it', async () => {
    const wrapper = mountEyepiece()
    const links: HTMLAnchorElement[] = []
    const realCreate = document.createElement.bind(document)
    vi.spyOn(document, 'createElement').mockImplementation((tag: string) => {
      const el = realCreate(tag)
      if (tag === 'a' && el instanceof HTMLAnchorElement) {
        el.click = () => { links.push(el) }
      }
      return el
    })

    await splitButton(wrapper).vm.$emit('click')
    await nextTick()

    expect(links).toHaveLength(1)
    expect(links[0].getAttribute('download')).toBe(mockSnapshotFilename)
    expect(links[0].href).toContain('blob:snapshot')
  })

  // The menu is teleported to `body`, so it neither fades with the controls nor
  // feeds them pointer events: without the hold, its button fades out underneath it.
  it('keeps the controls up while the menu is open', async () => {
    vi.useFakeTimers()
    const wrapper = mountEyepiece()

    await splitButton(wrapper).vm.$emit('menuToggle', true)
    vi.advanceTimersByTime(IDLE_HIDE_MS * 2)
    await nextTick()

    expect(wrapper.find('.eyepiece-overlay').classes()).not.toContain('overlay-hidden')
    vi.useRealTimers()
  })

  it('lets them fade again once the menu closes', async () => {
    vi.useFakeTimers()
    const wrapper = mountEyepiece()

    await splitButton(wrapper).vm.$emit('menuToggle', true)
    await splitButton(wrapper).vm.$emit('menuToggle', false)
    vi.advanceTimersByTime(IDLE_HIDE_MS)
    await nextTick()

    expect(wrapper.find('.eyepiece-overlay').classes()).toContain('overlay-hidden')
    vi.useRealTimers()
  })

  // A busy server is retried for up to fifteen seconds; the observer needs to see
  // that something is happening, and the spinner must not fade out halfway through.
  it('shows a spinner in place of the icon while downloading', async () => {
    let release: (answer: EyepieceSnapshot) => void = () => {}
    fetchEyepieceSnapshot.mockReturnValueOnce(new Promise((r) => (release = r)))
    const wrapper = mountEyepiece()

    await splitButton(wrapper).vm.$emit('click')
    await nextTick()

    expect(wrapper.findComponent({name: 'BaseSpinner'}).exists()).toBe(true)
    expect(wrapper.find('.download-btn .split-button-main svg').exists()).toBe(false)

    release(snapshot())
    await nextTick()
    await nextTick()
    expect(wrapper.findComponent({name: 'BaseSpinner'}).exists()).toBe(false)
  })

  it('keeps the controls up for the whole download', async () => {
    vi.useFakeTimers()
    let release: (answer: EyepieceSnapshot) => void = () => {}
    fetchEyepieceSnapshot.mockReturnValueOnce(new Promise((r) => (release = r)))
    const wrapper = mountEyepiece()

    await splitButton(wrapper).vm.$emit('click')
    await nextTick()
    vi.advanceTimersByTime(IDLE_HIDE_MS + 5000)
    await nextTick()

    expect(wrapper.find('.eyepiece-overlay').classes()).not.toContain('overlay-hidden')

    release(snapshot())
    vi.useRealTimers()
  })

  it('does not start a second download while one is running', async () => {
    let release: (answer: EyepieceSnapshot) => void = () => {}
    fetchEyepieceSnapshot.mockReturnValueOnce(new Promise((r) => (release = r)))
    const wrapper = mountEyepiece()

    await splitButton(wrapper).vm.$emit('click')
    await nextTick()
    await splitButton(wrapper).vm.$emit('select', 'original')
    await nextTick()

    expect(fetchEyepieceSnapshot).toHaveBeenCalledTimes(1)
    release(snapshot())
  })

  it('surfaces a failed download instead of failing silently', async () => {
    fetchEyepieceSnapshot.mockRejectedValueOnce(new Error('No frame to download yet.'))
    const wrapper = mountEyepiece()

    await splitButton(wrapper).vm.$emit('click')
    await nextTick()
    await nextTick()

    expect(wrapper.find('.download-error').text()).toBe('No frame to download yet.')
  })
})

describe('EyepieceView.vue Overlay layout', () => {
  beforeEach(async () => {
    vi.clearAllMocks()
    setPath('/eyepiece')
    installResizeObserverStub()
    mockCapabilities.value = capabilities({ debug_logging: true })
    await loadEyepieceView()
  })

  afterEach(() => {
    mockCapabilities.value = capabilities({ debug_logging: false })
    vi.resetModules()
  })

  // Both were pinned to the same corner, so the buttons sat on top of the readout.
  it('stacks the buttons above the backend readout instead of over it', async () => {
    const wrapper = mountEyepiece()
    await landFrame()

    const stack = wrapper.find('.eyepiece-overlay')
    const children = [...stack.element.children].map((el) => el.className)

    expect(wrapper.find('.debug-overlay').exists()).toBe(true)
    expect(children).toEqual(['eyepiece-controls', 'debug-overlay'])
  })
})

describe('EyepieceView.vue Overlay auto-hide', () => {
  beforeEach(async () => {
    vi.clearAllMocks()
    vi.useFakeTimers()
    setPath('/eyepiece')
    installResizeObserverStub()
    await loadEyepieceView()
  })

  afterEach(() => {
    vi.useRealTimers()
    vi.resetModules()
  })

  it('starts with the controls visible', () => {
    const wrapper = mountEyepiece()

    expect(wrapper.find('.eyepiece-overlay').classes()).not.toContain('overlay-hidden')
  })

  it('hides the controls once the idle countdown runs out after a tap', async () => {
    const wrapper = mountEyepiece()

    // Tap once to hide, once to show — then let the countdown run out.
    await wrapper.trigger('mousedown', {clientX: 5, clientY: 5})
    await wrapper.trigger('mouseup', {clientX: 5, clientY: 5})
    await wrapper.trigger('mousedown', {clientX: 5, clientY: 5})
    await wrapper.trigger('mouseup', {clientX: 5, clientY: 5})
    expect(wrapper.find('.eyepiece-overlay').classes()).not.toContain('overlay-hidden')

    vi.advanceTimersByTime(IDLE_HIDE_MS)
    await nextTick()

    expect(wrapper.find('.eyepiece-overlay').classes()).toContain('overlay-hidden')
  })

  it('toggles the controls away on a tap and back on the next one', async () => {
    const wrapper = mountEyepiece()

    await wrapper.trigger('mousedown', {clientX: 5, clientY: 5})
    await wrapper.trigger('mouseup', {clientX: 5, clientY: 5})
    expect(wrapper.find('.eyepiece-overlay').classes()).toContain('overlay-hidden')

    await wrapper.trigger('mousedown', {clientX: 5, clientY: 5})
    await wrapper.trigger('mouseup', {clientX: 5, clientY: 5})
    expect(wrapper.find('.eyepiece-overlay').classes()).not.toContain('overlay-hidden')
  })

  // The device the fade exists for. A browser replays a touch as compatibility
  // mouse events; handled naively one tap ran the toggle twice and nothing moved.
  it('toggles once for one tap, not once per synthesised mouse event', async () => {
    const wrapper = mountEyepiece()

    await wrapper.trigger('touchstart', {touches: [{clientX: 5, clientY: 5}]})
    await wrapper.trigger('touchend', {touches: [], changedTouches: [{clientX: 5, clientY: 5}]})
    await wrapper.trigger('mousedown', {clientX: 5, clientY: 5})
    await wrapper.trigger('mouseup', {clientX: 5, clientY: 5})

    expect(wrapper.find('.eyepiece-overlay').classes()).toContain('overlay-hidden')
  })

  it('does not toggle on touchcancel', async () => {
    const wrapper = mountEyepiece()

    await wrapper.trigger('touchstart', {touches: [{clientX: 5, clientY: 5}]})
    await wrapper.trigger('touchcancel', {touches: [], changedTouches: [{clientX: 5, clientY: 5}]})

    expect(wrapper.find('.eyepiece-overlay').classes()).not.toContain('overlay-hidden')
  })

  it('shows the controls when something inside them takes focus', async () => {
    const wrapper = mountEyepiece()
    await wrapper.trigger('mousedown', {clientX: 5, clientY: 5})
    await wrapper.trigger('mouseup', {clientX: 5, clientY: 5})
    expect(wrapper.find('.eyepiece-overlay').classes()).toContain('overlay-hidden')

    await wrapper.trigger('focusin')

    expect(wrapper.find('.eyepiece-overlay').classes()).not.toContain('overlay-hidden')
  })

  it('fades on its own if the view is never touched', async () => {
    const wrapper = mountEyepiece()

    vi.advanceTimersByTime(IDLE_HIDE_MS)
    await nextTick()

    expect(wrapper.find('.eyepiece-overlay').classes()).toContain('overlay-hidden')
  })

  // The Push-To chevrons are the one overlay that must survive the fade: they are
  // navigation, not chrome.
  it('never hides the Push-To chevrons', async () => {
    const wrapper = mount(EyepieceViewModule, {
      global: {
        provide: contextProvides({
          settings: eyepieceSettings({ binoview: false, circular_view: true }),
          eventStream: createEventStreamStub({
            pushDirection: clientDirection({ angleDeg: 45, distanceDeg: 10, isClose: false, directionHint: 'NE' }),
            currentTarget: catalogEntry(),
          }),
        }),
      },
    })

    await wrapper.trigger('mousedown', {clientX: 5, clientY: 5})
    await wrapper.trigger('mouseup', {clientX: 5, clientY: 5})
    vi.advanceTimersByTime(IDLE_HIDE_MS * 2)
    await nextTick()

    const arrow = wrapper.findComponent({ name: 'GuideArrow' })
    expect(arrow.exists()).toBe(true)
    expect(arrow.classes()).not.toContain('overlay-hidden')
  })
})

/**
 * The viewers follow the operator's Guide toggle on `/` and have no control of their own:
 * the server switches their socket, so the page only reflects what it is shown.
 */
describe('EyepieceView.vue follows the operator', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    installResizeObserverStub()
  })

  afterEach(() => {
    mockCapabilities.value = capabilities({ debug_logging: false })
    vi.resetModules()
  })

  for (const [route, socket] of [['/eyepiece', '/ws/eyepiece'], ['/eyepiece_quality', '/ws/eyepiece_quality']] as const) {
    it(`offers no camera control and names no source on ${route}`, async () => {
      setPath(route)
      await loadEyepieceView()
      const wrapper = mountEyepiece(undefined, { eventStream: { viewedCamera: 'guide' } })
      await landFrame()

      expect(useImageStream).toHaveBeenCalledWith({ endpoint: socket })
      expect(wrapper.find('.guide-toggle').exists()).toBe(false)
      const buttons = wrapper.findAll('button').map((b) => b.text())
      expect(buttons.some((text) => /guide|camera/i.test(text))).toBe(false)
    })
  }

  it('notes the guide camera in the readout while it is viewed', async () => {
    setPath('/eyepiece_quality')
    mockCapabilities.value = capabilities({ debug_logging: true })
    await loadEyepieceView()
    const viewedCamera = ref<CameraRole>('main')
    const wrapper = mountEyepiece(undefined, { eventStream: { viewedCamera } })
    await landFrame()
    expect(wrapper.find('.debug-overlay').text()).not.toContain('Guide camera')

    viewedCamera.value = 'guide'
    await nextTick()

    expect(wrapper.find('.debug-overlay').text()).toContain('Guide camera')
  })

  it("places the arrow against the guide camera's field while it is viewed", async () => {
    setPath('/eyepiece')
    await loadEyepieceView()
    const guideOptics = { focal_length_mm: 120, pixel_size_y_um: 2.9, sensor_height_px: 1080 }
    const imagingOptics = { focal_length_mm: 400, pixel_size_y_um: 3.76, sensor_height_px: 3008 }
    const telescope = settings().telescope
    const viewedCamera = ref<CameraRole>('guide')
    const wrapper = mountEyepiece(undefined, {
      settings: {
        telescope: { ...telescope, ...imagingOptics },
        camera_telescope_profiles: { 'Guide Cam': { ...telescope, ...guideOptics } },
      },
      eventStream: {
        viewedCamera,
        pushDirection: clientDirection({ angleDeg: 45, distanceDeg: 10, isClose: false, directionHint: 'NE', fovDeg: 0.5 }),
        currentTarget: catalogEntry(),
      },
      provide: {
        guideCamera: ref(camera({ name: 'Guide Cam' })),
        mainCamera: ref(camera({ name: 'Imaging Cam' })),
      },
    })
    const field = ({ focal_length_mm: fl, pixel_size_y_um: py, sensor_height_px: h }: typeof guideOptics) =>
      (2 * Math.atan((h * py) / 1000 / (2 * fl)) * 180) / Math.PI
    const arrowFov = () => wrapper.findComponent({ name: 'GuideArrow' }).props('fovDeg')

    expect(arrowFov()).toBeCloseTo(field(guideOptics), 6)
    viewedCamera.value = 'main'
    await nextTick()
    expect(arrowFov()).toBeCloseTo(field(imagingOptics), 6)
  })

  // 2026-10-24: the guide's 2712x1538 frame, a 60.1° push 10° off. `/` read 3.4 screens and
  // this page 6.9, counting its circle; both count frames of the camera image now.
  it('counts screens in frames on the circular view, as `/` does', async () => {
    setPath('/eyepiece_quality')
    await loadEyepieceView()
    const wrapper = mountEyepiece({ binoview: false, circular_view: true }, {
      eventStream: {
        viewedCamera: ref<CameraRole>('guide'),
        pushDirection: clientDirection({
          angleDeg: 60.14, distanceDeg: 10, isClose: false, directionHint: 'NW', fovDeg: 1.45, solvedCamera: 'Guide Cam',
        }),
        currentTarget: catalogEntry(),
      },
      provide: {
        guideCamera: ref(camera({ name: 'Guide Cam' })),
        mainCamera: ref(camera({ name: 'Imaging Cam' })),
      },
    })
    lastStream().dimensions.value = { width: 2712, height: 1538 }
    await nextTick()

    expect(wrapper.find('.screen-size').text()).toBe('3.4× screen')
  })

  it('keeps the chevron only on the camera whose solve oriented it', async () => {
    setPath('/eyepiece_quality')
    await loadEyepieceView()
    const viewedCamera = ref<CameraRole>('guide')
    const wrapper = mountEyepiece({ binoview: true, circular_view: true }, {
      eventStream: {
        viewedCamera,
        pushDirection: clientDirection({ angleDeg: 37, distanceDeg: 26.1, isClose: false, directionHint: 'NE', solvedCamera: 'Guide Cam' }),
        currentTarget: catalogEntry({ designation: 'NGC 40', name: 'Bow-Tie nebula', messier: null }),
      },
      provide: {
        guideCamera: ref(camera({ name: 'Guide Cam' })),
        mainCamera: ref(camera({ name: 'Imaging Cam' })),
      },
    })
    const others = () => wrapper.findAllComponents({ name: 'GuideArrow' }).map((a) => a.props('otherCamera'))

    expect(others()).toEqual([null, null])
    viewedCamera.value = 'main'
    await nextTick()
    const guide = { name: 'Guide Cam', connected: true }
    expect(others()).toEqual([guide, guide])
  })

  it('names no guide camera over the imaging one once the guide has disconnected', async () => {
    setPath('/eyepiece_quality')
    await loadEyepieceView()
    const wrapper = mountEyepiece({ binoview: true, circular_view: true }, {
      eventStream: {
        viewedCamera: ref<CameraRole>('main'),
        pushDirection: clientDirection({ angleDeg: 37, distanceDeg: 26.1, isClose: false, directionHint: 'NE', solvedCamera: 'Guide Cam' }),
        currentTarget: catalogEntry({ designation: 'NGC 40', name: 'Bow-Tie nebula', messier: null }),
      },
      provide: {
        guideCamera: ref(null),
        mainCamera: ref(camera({ name: 'Imaging Cam' })),
      },
    })

    const arrows = wrapper.findAllComponents({ name: 'GuideArrow' })
    expect(arrows.map((a) => a.props('otherCamera'))).toEqual(Array(2).fill({ name: 'Guide Cam', connected: false }))
    expect(wrapper.find('.other-camera').exists()).toBe(false)
    expect(wrapper.find('.arrow-wrapper').exists()).toBe(false)
  })

  /**
   * Off the circular view the canvas takes the frame's own aspect, so the guide camera's
   * 16:9 picture after a square imaging frame moves the canvas inside an eye that stays the
   * same size. The eye's ResizeObserver never fires; the arrow must still follow.
   */
  for (const jpeg of [false, true]) it(`re-measures the arrow bounds when a switch changes the frame size (${jpeg ? 'JPEG' : 'LZ4'})`, async () => {
    setPath('/eyepiece')
    await loadEyepieceView()
    // JPEG draws once the bitmap decodes, later than the frame lands.
    vi.stubGlobal('createImageBitmap', async () => {
      const { width, height } = stream.dimensions.value
      return { width, height, close() {} }
    })
    let stream: ImageStream
    let box = { left: 0, top: 0, width: 800, height: 800 }
    const props = {
      offsetLeft: () => box.left,
      offsetTop: () => box.top,
      offsetWidth: () => box.width,
      offsetHeight: () => box.height,
    }
    const saved: Record<string, PropertyDescriptor | undefined> = {}
    for (const [name, get] of Object.entries(props)) {
      saved[name] = Object.getOwnPropertyDescriptor(HTMLElement.prototype, name)
      Object.defineProperty(HTMLCanvasElement.prototype, name, { configurable: true, get })
    }
    try {
      const wrapper = mountEyepiece({ binoview: false, circular_view: false }, {
        eventStream: {
          viewedCamera: 'main',
          pushDirection: clientDirection({ angleDeg: 45, distanceDeg: 10, isClose: false, directionHint: 'NE', fovDeg: 1 }),
          currentTarget: catalogEntry(),
        },
      })
      stream = lastStream()
      stream.isJpeg.value = jpeg
      const arrowBox = () => {
        return wrapper.findComponent({ name: 'GuideArrow' }).props('bounds')
      }
      const arrowFrame = () => wrapper.findComponent({ name: 'GuideArrow' }).props('frame')
      const settle = () => new Promise((resolve) => setTimeout(resolve, 50))

      stream.dimensions.value = { width: 3008, height: 3008 }
      stream.frameData.value = jpeg ? new Blob() : new Uint8Array(4)
      await settle()
      expect(arrowBox()).toEqual(box)
      expect(arrowFrame()).toEqual({ width: 3008, height: 3008 })

      // The guide camera's first frame arrives on the same socket.
      box = { left: 0, top: 175, width: 800, height: 450 }
      stream.dimensions.value = { width: 1920, height: 1080 }
      stream.frameData.value = jpeg ? new Blob() : new Uint8Array(4)
      await settle()
      expect(arrowBox()).toEqual(box)
      expect(arrowFrame()).toEqual({ width: 1920, height: 1080 })
    } finally {
      for (const name of Object.keys(props)) {
        Reflect.deleteProperty(HTMLCanvasElement.prototype, name)
        const descriptor = saved[name]
        if (descriptor) Object.defineProperty(HTMLElement.prototype, name, descriptor)
      }
    }
  })
})

describe('EyepieceView.vue Teardown', () => {
  beforeEach(async () => {
    vi.clearAllMocks()
    setPath('/eyepiece_quality')
    installResizeObserverStub()
    await loadEyepieceView()
  })

  afterEach(() => {
    vi.resetModules()
  })

  it('leaves no timer behind when it goes away mid-layout', async () => {
    vi.useFakeTimers()
    const current = eyepieceSettings({ binoview: true, circular_view: true })
    const wrapper = mountEyepiece(undefined, { provide: { settings: current } })
    await landFrame()
    current.value = eyepieceSettings({ binoview: false, circular_view: true }).value
    await nextTick()

    wrapper.unmount()

    expect(vi.getTimerCount()).toBe(0)
  })
})

/**
 * The eyepiece has no status bar, so the compass over the chevrons is the only sign that
 * Push-To is solving rather than stopped.
 */
describe('EyepieceView.vue Push-To solving', () => {
  const SOLVING: PlateSolving = { ...NOT_SOLVING, inProgress: true, targetName: 'M31' }
  const POINTING = clientDirection({ angleDeg: 45, distanceDeg: 10, isClose: false, directionHint: 'NE' })

  beforeEach(async () => {
    vi.clearAllMocks()
    setPath('/eyepiece')
    installResizeObserverStub()
    await loadEyepieceView()
  })

  afterEach(() => {
    vi.resetModules()
  })

  it('shows the compass alone while a new target\'s first solve runs', () => {
    const wrapper = mountEyepiece(undefined, {
      eventStream: { currentTarget: catalogEntry(), plateSolving: SOLVING },
    })

    const arrow = wrapper.findComponent({ name: 'GuideArrow' })
    expect(arrow.props('direction')).toBeNull()
    expect(arrow.props('solving')).toBe(true)
    expect(wrapper.find('.push-to-solving').exists()).toBe(true)
  })

  it('shows nothing for a solve with no target', () => {
    const wrapper = mountEyepiece(undefined, { eventStream: { plateSolving: SOLVING } })

    expect(wrapper.findComponent({ name: 'GuideArrow' }).exists()).toBe(false)
  })

  it('turns the compass in place of the readout, and hands it the step', () => {
    const stage = { label: 'blind', attempt: 2, total: 3 }
    const wrapper = mountEyepiece(undefined, {
      eventStream: { currentTarget: catalogEntry(), pushDirection: POINTING, plateSolving: { ...SOLVING, stage } },
    })

    expect(wrapper.findComponent({ name: 'GuideArrow' }).props('solveStage')).toEqual(stage)
    expect(wrapper.find('.arrow-wrapper').exists()).toBe(true)
    expect(wrapper.find('.step').text()).toBe('step 2/3')
    expect(wrapper.find('.distance').exists()).toBe(false)
  })

  it('keeps the compass one turn after the solve ends, then gives the readout back', async () => {
    vi.useFakeTimers()
    const plateSolving = ref<PlateSolving>(SOLVING)
    const wrapper = mountEyepiece(undefined, {
      eventStream: { currentTarget: catalogEntry(), pushDirection: POINTING, plateSolving },
    })

    plateSolving.value = { ...NOT_SOLVING, targetName: 'M31', lastResult: 'success' }
    await nextTick()
    vi.advanceTimersByTime(SOLVING_HOLD_MS - 1)
    await nextTick()
    expect(wrapper.find('.push-to-solving').exists()).toBe(true)

    vi.advanceTimersByTime(1)
    await nextTick()
    expect(wrapper.find('.push-to-solving').exists()).toBe(false)
    expect(wrapper.find('.distance').exists()).toBe(true)
  })

  it('drops a first solve\'s compass once the hold is over, when the solve found nothing', async () => {
    vi.useFakeTimers()
    const plateSolving = ref<PlateSolving>(SOLVING)
    const wrapper = mountEyepiece(undefined, {
      eventStream: { currentTarget: catalogEntry(), plateSolving },
    })

    plateSolving.value = { ...NOT_SOLVING, targetName: 'M31', lastResult: 'failed' }
    await nextTick()
    // Longer than after a success: the next attempt follows about 2 s after a failure.
    vi.advanceTimersByTime(FAILED_SOLVE_HOLD_MS - 1)
    await nextTick()
    expect(wrapper.find('.push-to-solving').exists()).toBe(true)

    vi.advanceTimersByTime(1)
    await nextTick()
    expect(wrapper.findComponent({ name: 'GuideArrow' }).exists()).toBe(false)
  })

  it('dims the readout as the last fix once a solve failed, in each eye', async () => {
    setPath('/eyepiece_quality')
    await loadEyepieceView()
    const wrapper = mountEyepiece({ binoview: true, circular_view: true }, {
      eventStream: {
        currentTarget: catalogEntry(),
        pushDirection: { ...POINTING, stale: true },
        plateSolving: { ...NOT_SOLVING, targetName: 'M31', lastResult: 'failed' },
      },
    })

    expect(wrapper.findAll('.last-fix')).toHaveLength(2)
    expect(wrapper.findAll('.distance-info.stale')).toHaveLength(2)
  })

  // `no_frame` on a camera switch zeroes the stream; the count measures the sensor till a frame lands.
  it('hands the arrow the viewed camera\'s sensor while the stream has no frame', async () => {
    const wrapper = mountEyepiece(undefined, {
      eventStream: { viewedCamera: ref<CameraRole>('guide'), currentTarget: catalogEntry(), pushDirection: POINTING },
      provide: {
        guideCamera: ref(camera({ name: 'Guide Cam', info: { max_width: 2712, max_height: 1538 } })),
        mainCamera: ref(camera({ name: 'Imaging Cam' })),
      },
    })
    const arrowFrame = () => wrapper.findComponent({ name: 'GuideArrow' }).props('frame')

    expect(arrowFrame()).toEqual({ width: 2712, height: 1538 })
    lastStream().dimensions.value = { width: 1356, height: 769 }
    await nextTick()
    expect(arrowFrame()).toEqual({ width: 1356, height: 769 })
  })

  it('turns a compass in each eye of a binoview', async () => {
    setPath('/eyepiece_quality')
    await loadEyepieceView()
    const wrapper = mountEyepiece({ binoview: true, circular_view: true }, {
      eventStream: { currentTarget: catalogEntry(), plateSolving: SOLVING },
    })

    expect(wrapper.find('.left-eye .push-to-solving').exists()).toBe(true)
    expect(wrapper.find('.right-eye .push-to-solving').exists()).toBe(true)
  })
})
