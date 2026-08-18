import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { ref } from 'vue'

// Mock useWebSocket composables
vi.mock('../../composables/useWebSocket.js', () => ({
  useImageStream: vi.fn(() => ({
    connected: ref(true),
    frameData: ref(null),
    dimensions: ref({ width: 0, height: 0 }),
  }))
}))

// Mock renderer composables
vi.mock('../../composables/useWebGLRenderer.js', () => ({
  useWebGLRenderer: vi.fn(() => ({
    init: vi.fn(() => true),
    render: vi.fn(),
    cleanup: vi.fn(),
    isInitialized: vi.fn(() => true),
  }))
}))

vi.mock('../../composables/useCanvas2DRenderer.js', () => ({
  useCanvas2DRenderer: vi.fn(() => ({
    init: vi.fn(() => false),
    render: vi.fn(),
    cleanup: vi.fn(),
    isInitialized: vi.fn(() => false),
  }))
}))

import { useImageStream } from '../../composables/useWebSocket.js'

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

    expect(useImageStream).toHaveBeenCalledWith({ endpoint: '/ws/eyepiece' })
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

    expect(useImageStream).toHaveBeenCalledWith({ endpoint: '/ws/eyepiece_quality' })
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
