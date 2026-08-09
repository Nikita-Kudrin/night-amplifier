import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { ref } from 'vue'

// Mock composables
vi.mock('./composables/useAppState.js', () => ({
  useAppState: vi.fn(() => ({
    loading: ref(false),
    globalError: ref(null),
    simulatorEnabled: ref(false),
    settings: ref({ eula_accepted: true }),
    refreshSettings: vi.fn(),
    refreshCameras: vi.fn(),
    initializeState: vi.fn(),
    updateCameraStatus: vi.fn(),
    updateCameraPhase: vi.fn(),
    addDiscoveredCamera: vi.fn(),
    _settingsRef: ref({}),
    _camerasRef: ref([]),
    _selectedCameraIdRef: ref(null),
    _cameraStatusRef: ref(null),
    _cameraPhaseRef: ref(null),
    capabilities: ref({}),
  }))
}))

vi.mock('./composables/useWebSocket.js', () => ({
  useEventStream: vi.fn(() => ({
    lastEvent: ref(null),
  }))
}))

vi.mock('./composables/api.js', () => ({
  getAstapStatus: vi.fn().mockResolvedValue({ ready: true }),
  getCatalogStatus: vi.fn().mockResolvedValue({ installed: true }),
}))

// We must mock App.vue import so that window.location is set BEFORE module is evaluated
// But we can also just use vi.doMock or isolateModules if needed.
// However, since we're using Vite/Vitest, we can manipulate window.location and dynamically import App.vue
describe('App.vue Routing', () => {
  let originalLocation

  beforeEach(() => {
    originalLocation = window.location
    delete window.location
  })

  afterEach(() => {
    window.location = originalLocation
    vi.resetModules()
  })

  it('renders EyepieceView on /eyepiece', async () => {
    window.location = { ...originalLocation, pathname: '/eyepiece' }
    const App = (await import('./App.vue')).default
    
    const wrapper = mount(App, {
      global: {
        stubs: {
          EyepieceView: true,
          EulaModal: true,
          StatusBar: true,
          LiveView: true,
          CameraPanel: true,
          CaptureControls: true,
          SettingsPanel: true,
        }
      }
    })

    expect(wrapper.findComponent({ name: 'EyepieceView' }).exists()).toBe(true)
  })

  it('renders EyepieceView on /eyepiece_quality', async () => {
    window.location = { ...originalLocation, pathname: '/eyepiece_quality' }
    const App = (await import('./App.vue')).default
    
    const wrapper = mount(App, {
      global: {
        stubs: {
          EyepieceView: true,
          EulaModal: true,
          StatusBar: true,
          LiveView: true,
          CameraPanel: true,
          CaptureControls: true,
          SettingsPanel: true,
        }
      }
    })

    expect(wrapper.findComponent({ name: 'EyepieceView' }).exists()).toBe(true)
  })

  it('does not render EyepieceView on root /', async () => {
    window.location = { ...originalLocation, pathname: '/' }
    const App = (await import('./App.vue')).default
    
    const wrapper = mount(App, {
      global: {
        stubs: {
          EyepieceView: true,
          EulaModal: true,
          StatusBar: true,
          LiveView: true,
          CameraPanel: true,
          CaptureControls: true,
          SettingsPanel: true,
        }
      }
    })

    expect(wrapper.findComponent({ name: 'EyepieceView' }).exists()).toBe(false)
    expect(wrapper.find('.app').exists()).toBe(true)
  })
})
