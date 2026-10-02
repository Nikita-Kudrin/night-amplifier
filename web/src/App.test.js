import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
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
    replaceCameraPhases: vi.fn(),
    addDiscoveredCamera: vi.fn(),
    _settingsRef: ref({}),
    _camerasRef: ref([]),
    _selectedCameraIdRef: ref(null),
    _cameraStatusRef: ref(null),
    _cameraPhaseRef: ref(null),
    _warmupEndsAtRef: ref({}),
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
  getAiCompute: vi.fn().mockResolvedValue({ state: 'unavailable', rungs: [] }),
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

  /** The one-time AI compute benchmark blocks the whole UI until it ends. */
  it('makes the app inert and shows the benchmark overlay while benchmarking', async () => {
    window.location = { ...originalLocation, pathname: '/' }
    const api = await import('./composables/api.js')
    api.getAiCompute.mockResolvedValueOnce({ state: 'benchmarking', rungs: [], progress: { done: 0, total: 2, testing: 'CPU — test' } })
    const App = (await import('./App.vue')).default

    const wrapper = mount(App, {
      attachTo: document.body,
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
    await flushPromises()

    expect(wrapper.find('.app').attributes('inert')).toBeDefined()
    expect(document.querySelector('[data-test="benchmark-overlay"]')).not.toBeNull()
    wrapper.unmount()
  })

  // Sent when the event socket (re)connects. A page that was away also missed cameras
  // connecting and disconnecting, so a snapshot naming other cameras refetches the list.
  it('takes phases from the server snapshot and refetches a camera list that disagrees', async () => {
    window.location = { ...originalLocation, pathname: '/' }
    const App = (await import('./App.vue')).default
    const { useAppState } = await import('./composables/useAppState.js')
    const { useEventStream } = await import('./composables/useWebSocket.js')
    mount(App, { global: { stubs: { StatusBar: true, LiveView: true, CameraPanel: true, CaptureControls: true, SettingsPanel: true } } })
    const appState = useAppState.mock.results.at(-1).value
    const events = useEventStream.mock.results.at(-1).value

    const cameras = [{ name: 'Neptune-C II', role: 'guide', phase: 'guiding' }]
    events.lastEvent.value = { type: 'camera_phases', cameras }
    await flushPromises()

    expect(appState.replaceCameraPhases).toHaveBeenCalledWith(cameras)
    expect(appState.refreshCameras).toHaveBeenCalledTimes(1)

    appState._camerasRef.value = [{ name: 'Neptune-C II', connected: true }]
    events.lastEvent.value = { type: 'camera_phases', cameras: [...cameras] }
    await flushPromises()
    expect(appState.refreshCameras).toHaveBeenCalledTimes(1)
  })

  it('leaves the app interactive when there is no benchmark', async () => {
    window.location = { ...originalLocation, pathname: '/' }
    const App = (await import('./App.vue')).default
    const wrapper = mount(App, { global: { stubs: { StatusBar: true, LiveView: true, CameraPanel: true, CaptureControls: true, SettingsPanel: true } } })
    await flushPromises()
    expect(wrapper.find('.app').attributes('inert')).toBeUndefined()
    expect(wrapper.find('[data-test="benchmark-overlay"]').exists()).toBe(false)
  })
})
