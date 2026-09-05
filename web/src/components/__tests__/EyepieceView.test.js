import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { nextTick, ref } from 'vue'

// Mock useWebSocket composables
vi.mock('../../composables/useWebSocket.js', () => ({
  useImageStream: vi.fn(() => ({
    connected: ref(true),
    frameData: ref(null),
    dimensions: ref({ width: 0, height: 0 }),
    isJpeg: ref(false),
    sendResolution: vi.fn(),
  }))
}))

// Mock renderer composables
vi.mock('../../composables/useWebGLRenderer.js', () => ({
  useWebGLRenderer: vi.fn(() => ({
    init: vi.fn(() => true),
    render: vi.fn(),
    cleanup: vi.fn(),
    isInitialized: vi.fn(() => true),
    backend: ref('webgl2-8bit'),
  }))
}))

vi.mock('../../composables/useCanvas2DRenderer.js', () => ({
  useCanvas2DRenderer: vi.fn(() => ({
    init: vi.fn(() => false),
    render: vi.fn(),
    cleanup: vi.fn(),
    isInitialized: vi.fn(() => false),
    backend: ref('canvas2d'),
  }))
}))

const mockSnapshotFilename = 'eyepiece_05-09-2026_21-14-03.png'
vi.mock('../../composables/api.js', () => ({
  fetchEyepieceSnapshot: vi.fn(async () => ({
    blob: new Blob(['png'], { type: 'image/png' }),
    filename: mockSnapshotFilename,
  })),
}))

/** The backend readout is debug-only, so its layout can only be tested with it on. */
const mockCapabilities = ref({ debug_logging: false })
vi.mock('../../composables/useAppState.js', () => ({
  getAppState: () => ({ capabilities: mockCapabilities }),
}))

import { useImageStream } from '../../composables/useWebSocket.js'
import { fetchEyepieceSnapshot } from '../../composables/api.js'
import { IDLE_HIDE_MS } from '../../composables/useOverlayVisibility.js'

function mountEyepiece(settings = { binoview: true, circular_view: true }, overrides = {}) {
  return mount(EyepieceViewModule, {
    global: {
      provide: {
        settings: ref({ eyepiece: settings, ...overrides.settings }),
        eventStream: {
          pushDirection: ref(null),
          currentTarget: ref(null),
        },
      },
    },
  })
}

/**
 * Land a frame on the most recently mounted view. Zoom, the download buttons and
 * the placeholder all gate on having one.
 */
async function landFrame() {
  const results = useImageStream.mock.results
  const stream = results[results.length - 1].value
  stream.dimensions.value = { width: 100, height: 100 }
  stream.frameData.value = new Uint8Array(4)
  await nextTick()
}

/** Assigned by each `beforeEach` that needs a component; see `loadEyepieceView`. */
let EyepieceViewModule

async function loadEyepieceView() {
  EyepieceViewModule = (await import('../EyepieceView.vue')).default
  return EyepieceViewModule
}

describe('EyepieceView.vue Endpoint Selection', () => {
  let originalLocation

  beforeEach(() => {
    vi.clearAllMocks()
    originalLocation = window.location
    delete window.location
  })

  afterEach(() => {
    window.location = originalLocation
    vi.resetModules()
  })

  it('uses /ws/eyepiece when path is /eyepiece (JPEG streaming)', async () => {
    window.location = { ...originalLocation, pathname: '/eyepiece' }
    
    // We import dynamically to ensure it runs after window.location is set
    const EyepieceView = (await import('../EyepieceView.vue')).default
    mount(EyepieceView, {
      global: {
        provide: {
          settings: ref({ eyepiece: { binoview: true, circular_view: true } }),
          eventStream: {
            pushDirection: ref(null),
            currentTarget: ref(null),
          }
        }
      }
    })

    expect(useImageStream).toHaveBeenCalledWith({
      endpoint: '/ws/eyepiece',
      width: Math.round(window.innerWidth * (window.devicePixelRatio || 1)),
      height: Math.round(window.innerHeight * (window.devicePixelRatio || 1)),
    })
  })

  it('uses /ws/eyepiece_quality when path is /eyepiece_quality (raw 8bit + LZ4)', async () => {
    window.location = { ...originalLocation, pathname: '/eyepiece_quality' }
    
    const EyepieceView = (await import('../EyepieceView.vue')).default
    mount(EyepieceView, {
      global: {
        provide: {
          settings: ref({ eyepiece: { binoview: true, circular_view: true } }),
          eventStream: {
            pushDirection: ref(null),
            currentTarget: ref(null),
          }
        }
      }
    })

    expect(useImageStream).toHaveBeenCalledWith({
      endpoint: '/ws/eyepiece_quality',
      width: Math.round(window.innerWidth * (window.devicePixelRatio || 1)),
      height: Math.round(window.innerHeight * (window.devicePixelRatio || 1)),
    })
  })

  it('sends resolution updates on window resize', async () => {
    vi.useFakeTimers()
    window.location = { ...originalLocation, pathname: '/eyepiece' }
    window.innerWidth = 800
    window.innerHeight = 600
    window.devicePixelRatio = 2
    
    const EyepieceView = (await import('../EyepieceView.vue')).default
    mount(EyepieceView, {
      global: {
        provide: {
          settings: ref({ eyepiece: { binoview: true, circular_view: true } }),
          eventStream: {
            pushDirection: ref(null),
            currentTarget: ref(null),
          }
        }
      }
    })

    const { sendResolution } = useImageStream.mock.results[0].value
    
    // Simulate resize
    window.innerWidth = 1000
    window.innerHeight = 800
    window.dispatchEvent(new Event('resize'))
    
    // Fast forward debounce timer
    vi.advanceTimersByTime(250)
    
    // No canvas layout in jsdom, so this exercises the window fallback.
    expect(sendResolution).toHaveBeenCalledWith(2000, 1600) // 1000 * 2, 800 * 2
    vi.useRealTimers()
  })

  // The reason the report is canvas-derived rather than window-derived: in
  // binoview each eye canvas shows the whole frame at roughly half the window
  // width, so reporting the window would have the server send twice the pixels
  // either eye can display and leave the GPU to minify the rest away.
  it('reports the per-eye canvas size in binoview, not the window size', async () => {
    window.location = { ...originalLocation, pathname: '/eyepiece_quality' }
    window.innerWidth = 1440
    window.innerHeight = 1440
    window.devicePixelRatio = 1

    // jsdom reports 0 for every element's offset size; stand in for a laid-out
    // binoview where each eye canvas is half the window wide.
    const offsetWidth = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'offsetWidth')
    const offsetHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'offsetHeight')
    Object.defineProperty(HTMLElement.prototype, 'offsetWidth', {configurable: true, get: () => 720})
    Object.defineProperty(HTMLElement.prototype, 'offsetHeight', {configurable: true, get: () => 720})

    try {
      const EyepieceView = (await import('../EyepieceView.vue')).default
      mount(EyepieceView, {
        global: {
          provide: {
            settings: ref({ eyepiece: { binoview: true, circular_view: true } }),
            eventStream: {
              pushDirection: ref(null),
              currentTarget: ref(null),
            }
          }
        }
      })

      const { sendResolution } = useImageStream.mock.results[0].value
      expect(sendResolution).toHaveBeenCalledWith(720, 720)
      expect(sendResolution).not.toHaveBeenCalledWith(1440, 1440)
    } finally {
      Object.defineProperty(HTMLElement.prototype, 'offsetWidth', offsetWidth)
      Object.defineProperty(HTMLElement.prototype, 'offsetHeight', offsetHeight)
    }
  })

  // The de-duplication that used to live here moved into `useImageStream`. It is
  // the only layer that can tell "same size, same socket" (skip) from "same size,
  // new socket" (must re-send) — and a memo here suppressed exactly the report a
  // reconnect needs, pinning the lossless stream to the server's default tier for
  // the rest of the session.
  it('keeps reporting the viewport so a reconnect can replay it', async () => {
    vi.useFakeTimers()
    window.location = { ...originalLocation, pathname: '/eyepiece_quality' }
    window.innerWidth = 1440
    window.innerHeight = 1440
    window.devicePixelRatio = 1

    const EyepieceView = (await import('../EyepieceView.vue')).default
    mount(EyepieceView, {
      global: {
        provide: {
          settings: ref({ eyepiece: { binoview: true, circular_view: true } }),
          eventStream: {
            pushDirection: ref(null),
            currentTarget: ref(null),
          }
        }
      }
    })

    const { sendResolution } = useImageStream.mock.results[0].value
    const afterMount = sendResolution.mock.calls.length

    window.dispatchEvent(new Event('resize'))
    vi.advanceTimersByTime(250)

    expect(sendResolution.mock.calls.length).toBeGreaterThan(afterMount)
    expect(sendResolution).toHaveBeenLastCalledWith(1440, 1440)
    vi.useRealTimers()
  })
})

describe('EyepieceView.vue Layout', () => {
  let originalLocation

  beforeEach(() => {
    vi.clearAllMocks()
    originalLocation = window.location
    delete window.location
    window.location = { ...originalLocation, pathname: '/eyepiece' }
  })

  afterEach(() => {
    window.location = originalLocation
    vi.resetModules()
  })

  it('applies circular class to canvas when circular_view setting is true', async () => {
    const EyepieceView = (await import('../EyepieceView.vue')).default
    const wrapper = mount(EyepieceView, {
      global: {
        provide: {
          settings: ref({ eyepiece: { binoview: false, circular_view: true } }),
          eventStream: {
            pushDirection: ref(null),
            currentTarget: ref(null),
          }
        }
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
        provide: {
          settings: ref({ eyepiece: { binoview: false, circular_view: false } }),
          eventStream: {
            pushDirection: ref(null),
            currentTarget: ref(null),
          }
        }
      }
    })

    const singleCanvas = wrapper.find('.single-view canvas')
    expect(singleCanvas.exists()).toBe(true)
    expect(singleCanvas.classes()).not.toContain('circular')
  })

  it('renders GuideArrow when pushDirection and currentTarget are present', async () => {
    // We need to mock ResizeObserver
    global.ResizeObserver = class ResizeObserver {
      observe() {}
      unobserve() {}
      disconnect() {}
    }

    const EyepieceView = (await import('../EyepieceView.vue')).default
    const wrapper = mount(EyepieceView, {
      global: {
        provide: {
          settings: ref({ eyepiece: { binoview: true, circular_view: true } }),
          eventStream: {
            pushDirection: ref({ angleDeg: 45, distanceDeg: 10, isClose: false, directionHint: 'NE' }),
            currentTarget: ref({ id: 'M42', name: 'Orion Nebula' }),
          }
        }
      }
    })

    // 2 in binoview, 1 in single-view (which is hidden by v-show on the parent)
    const arrows = wrapper.findAllComponents({ name: 'GuideArrow' })
    expect(arrows.length).toBe(3)
  })

  it('does not render GuideArrow when pushDirection is null', async () => {
    global.ResizeObserver = class ResizeObserver {
      observe() {}
      unobserve() {}
      disconnect() {}
    }

    const EyepieceView = (await import('../EyepieceView.vue')).default
    const wrapper = mount(EyepieceView, {
      global: {
        provide: {
          settings: ref({ eyepiece: { binoview: false, circular_view: true } }),
          eventStream: {
            pushDirection: ref(null),
            currentTarget: ref({ id: 'M42', name: 'Orion Nebula' }),
          }
        }
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
  let originalLocation

  beforeEach(() => {
    vi.clearAllMocks()
    originalLocation = window.location
    delete window.location
    global.ResizeObserver = class ResizeObserver {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
  })

  afterEach(() => {
    window.location = originalLocation
    vi.resetModules()
  })

  /** `v-show` keeps both layouts mounted, so the hidden one is the one display:none'd. */
  function isHidden(wrapper, selector) {
    return (wrapper.find(selector).attributes('style') || '').includes('display: none')
  }

  it('stays monocular on /eyepiece even with binoview enabled', async () => {
    window.location = { ...originalLocation, pathname: '/eyepiece' }
    await loadEyepieceView()

    const wrapper = mountEyepiece({ binoview: true, circular_view: true })

    expect(isHidden(wrapper, '.binoview-container')).toBe(true)
    expect(isHidden(wrapper, '.single-view')).toBe(false)
  })

  it('honours binoview on /eyepiece_quality', async () => {
    window.location = { ...originalLocation, pathname: '/eyepiece_quality' }
    await loadEyepieceView()

    const wrapper = mountEyepiece({ binoview: true, circular_view: true })

    expect(isHidden(wrapper, '.binoview-container')).toBe(false)
    expect(isHidden(wrapper, '.single-view')).toBe(true)
  })

  it('is monocular on /eyepiece_quality when binoview is off', async () => {
    window.location = { ...originalLocation, pathname: '/eyepiece_quality' }
    await loadEyepieceView()

    const wrapper = mountEyepiece({ binoview: false, circular_view: true })

    expect(isHidden(wrapper, '.binoview-container')).toBe(true)
    expect(isHidden(wrapper, '.single-view')).toBe(false)
  })
})

describe('EyepieceView.vue Fullscreen', () => {
  let originalLocation

  beforeEach(async () => {
    vi.clearAllMocks()
    originalLocation = window.location
    delete window.location
    window.location = { ...originalLocation, pathname: '/eyepiece' }
    global.ResizeObserver = class ResizeObserver {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
    Element.prototype.requestFullscreen = vi.fn().mockResolvedValue(undefined)
    document.exitFullscreen = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(document, 'fullscreenElement', {
      value: null,
      writable: true,
      configurable: true,
    })
    Object.defineProperty(document, 'fullscreenEnabled', {
      value: true,
      writable: true,
      configurable: true,
    })
    await loadEyepieceView()
  })

  afterEach(() => {
    window.location = originalLocation
    vi.resetModules()
  })

  function fullscreenButton(wrapper) {
    return wrapper.findAll('.eyepiece-btn').find((b) => b.attributes('title')?.includes('ullscreen'))
  }

  it('renders a fullscreen button', () => {
    const wrapper = mountEyepiece()

    expect(fullscreenButton(wrapper)).toBeTruthy()
  })

  // iPhone Safari implements no Fullscreen API, so the button would be a control
  // that visibly does nothing on a phone at the eyepiece.
  it('hides the button where the browser has no fullscreen', async () => {
    document.fullscreenEnabled = false
    try {
      const wrapper = mountEyepiece()
      expect(fullscreenButton(wrapper)).toBeUndefined()
    } finally {
      document.fullscreenEnabled = true
    }
  })

  it('requests fullscreen when clicked', async () => {
    const wrapper = mountEyepiece()

    await fullscreenButton(wrapper).trigger('click')

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

    document.fullscreenElement = document.body
    document.dispatchEvent(new Event('fullscreenchange'))
    await nextTick()

    expect(wrapper.find('.fit-all-btn').exists()).toBe(false)
  })

  it('fits all on a rotation while fullscreen', async () => {
    vi.useFakeTimers()
    const wrapper = mountEyepiece()
    await landFrame()
    document.fullscreenElement = document.body
    document.dispatchEvent(new Event('fullscreenchange'))
    await wrapper.trigger('wheel', {deltaY: -2000, clientX: 100, clientY: 100})
    expect(wrapper.find('.fit-all-btn').exists()).toBe(true)

    window.dispatchEvent(new Event('orientationchange'))
    vi.advanceTimersByTime(250)
    await nextTick()

    expect(wrapper.find('.fit-all-btn').exists()).toBe(false)
    vi.useRealTimers()
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
  let originalLocation

  beforeEach(async () => {
    vi.clearAllMocks()
    originalLocation = window.location
    delete window.location
    window.location = { ...originalLocation, pathname: '/eyepiece' }
    global.ResizeObserver = class ResizeObserver {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
    global.URL.createObjectURL = vi.fn(() => 'blob:snapshot')
    global.URL.revokeObjectURL = vi.fn()
    await loadEyepieceView()
  })

  afterEach(() => {
    window.location = originalLocation
    vi.resetModules()
  })

  function splitButton(wrapper) {
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
    const links = []
    const realCreate = document.createElement.bind(document)
    vi.spyOn(document, 'createElement').mockImplementation((tag) => {
      const el = realCreate(tag)
      if (tag === 'a') {
        el.click = () => links.push(el)
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
    let release
    fetchEyepieceSnapshot.mockReturnValueOnce(new Promise((r) => (release = r)))
    const wrapper = mountEyepiece()

    await splitButton(wrapper).vm.$emit('click')
    await nextTick()

    expect(wrapper.findComponent({name: 'BaseSpinner'}).exists()).toBe(true)
    expect(wrapper.find('.download-btn .split-button-main svg').exists()).toBe(false)

    release(new Blob(['png'], {type: 'image/png'}))
    await nextTick()
    await nextTick()
    expect(wrapper.findComponent({name: 'BaseSpinner'}).exists()).toBe(false)
  })

  it('keeps the controls up for the whole download', async () => {
    vi.useFakeTimers()
    let release
    fetchEyepieceSnapshot.mockReturnValueOnce(new Promise((r) => (release = r)))
    const wrapper = mountEyepiece()

    await splitButton(wrapper).vm.$emit('click')
    await nextTick()
    vi.advanceTimersByTime(IDLE_HIDE_MS + 5000)
    await nextTick()

    expect(wrapper.find('.eyepiece-overlay').classes()).not.toContain('overlay-hidden')

    release(new Blob(['png'], {type: 'image/png'}))
    vi.useRealTimers()
  })

  it('does not start a second download while one is running', async () => {
    let release
    fetchEyepieceSnapshot.mockReturnValueOnce(new Promise((r) => (release = r)))
    const wrapper = mountEyepiece()

    await splitButton(wrapper).vm.$emit('click')
    await nextTick()
    await splitButton(wrapper).vm.$emit('select', 'original')
    await nextTick()

    expect(fetchEyepieceSnapshot).toHaveBeenCalledTimes(1)
    release(new Blob(['png'], {type: 'image/png'}))
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
  let originalLocation

  beforeEach(async () => {
    vi.clearAllMocks()
    originalLocation = window.location
    delete window.location
    window.location = { ...originalLocation, pathname: '/eyepiece' }
    global.ResizeObserver = class ResizeObserver {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
    mockCapabilities.value = { debug_logging: true }
    await loadEyepieceView()
  })

  afterEach(() => {
    window.location = originalLocation
    mockCapabilities.value = { debug_logging: false }
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
  let originalLocation

  beforeEach(async () => {
    vi.clearAllMocks()
    vi.useFakeTimers()
    originalLocation = window.location
    delete window.location
    window.location = { ...originalLocation, pathname: '/eyepiece' }
    global.ResizeObserver = class ResizeObserver {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
    await loadEyepieceView()
  })

  afterEach(() => {
    window.location = originalLocation
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
        provide: {
          settings: ref({ eyepiece: { binoview: false, circular_view: true } }),
          eventStream: {
            pushDirection: ref({ angleDeg: 45, distanceDeg: 10, isClose: false, directionHint: 'NE' }),
            currentTarget: ref({ id: 'M42', name: 'Orion Nebula' }),
          },
        },
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
