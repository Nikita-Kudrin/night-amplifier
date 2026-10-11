import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { ref, h, onMounted } from 'vue'

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

vi.mock('./composables/useEventStream.js', () => ({
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
          PushToPanel: true,
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
          PushToPanel: true,
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
          PushToPanel: true,
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
          PushToPanel: true,
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
    const { useEventStream } = await import('./composables/useEventStream.js')
    mount(App, { global: { stubs: { StatusBar: true, LiveView: true, CameraPanel: true, CaptureControls: true, SettingsPanel: true, PushToPanel: true } } })
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
    const wrapper = mount(App, { global: { stubs: { StatusBar: true, LiveView: true, CameraPanel: true, CaptureControls: true, SettingsPanel: true, PushToPanel: true } } })
    await flushPromises()
    expect(wrapper.find('.app').attributes('inert')).toBeUndefined()
    expect(wrapper.find('[data-test="benchmark-overlay"]').exists()).toBe(false)
  })
})

/** A stub panel that counts how often it mounts. */
function countingStub(name, mounts) {
  return {
    name,
    setup() {
      onMounted(() => { mounts[name] = (mounts[name] ?? 0) + 1 })
      return () => h('div', { class: `stub-${name}` })
    },
  }
}

describe('App.vue sidebar tabs', () => {
  let mounts
  let wrapper

  beforeEach(() => {
    mounts = {}
  })

  afterEach(() => {
    wrapper?.unmount()
    vi.resetModules()
  })

  async function mountApp() {
    const App = (await import('./App.vue')).default
    const { useAppState } = await import('./composables/useAppState.js')
    wrapper = mount(App, {
      attachTo: document.body,
      global: {
        stubs: {
          StatusBar: true,
          LiveView: true,
          CameraPanel: countingStub('CameraPanel', mounts),
          CaptureControls: countingStub('CaptureControls', mounts),
          PushToPanel: countingStub('PushToPanel', mounts),
          SettingsPanel: countingStub('SettingsPanel', mounts),
          PushToSetupOverlay: {
            name: 'PushToSetupOverlay',
            emits: ['installed', 'close'],
            setup: (_, { emit }) => () => h('button', { 'data-test': 'setup-installed', onClick: () => emit('installed') }),
          },
        },
      },
    })
    await flushPromises()
    return { appState: useAppState.mock.results.at(-1).value }
  }

  const pane = (name) => wrapper.find(`[data-test="sidebar-${name}"]`)
  const button = (name) => wrapper.find(`[data-test="tab-${name}"]`)

  async function click(name) {
    await button(name).trigger('click')
    await flushPromises()
  }

  it('shows only the camera and capture controls at first, mounting no other tab', async () => {
    await mountApp()

    expect(pane('main').isVisible()).toBe(true)
    expect(wrapper.find('.stub-CameraPanel').isVisible()).toBe(true)
    expect(wrapper.find('.stub-CaptureControls').isVisible()).toBe(true)
    expect(pane('push-to').exists()).toBe(false)
    expect(pane('settings').exists()).toBe(false)
    expect(button('settings').attributes('aria-pressed')).toBe('false')
    expect(button('push-to').attributes('aria-pressed')).toBe('false')
    expect(wrapper.find('.sidebar').classes()).not.toContain('expanded')
  })

  it('shows Settings alone, and its button again brings the main controls back', async () => {
    await mountApp()

    await click('settings')
    expect(pane('settings').isVisible()).toBe(true)
    expect(pane('main').isVisible()).toBe(false)
    expect(wrapper.find('.stub-CameraPanel').isVisible()).toBe(false)
    expect(wrapper.find('.stub-CaptureControls').isVisible()).toBe(false)
    expect(button('settings').attributes('aria-pressed')).toBe('true')
    expect(button('settings').classes()).toContain('active')
    expect(wrapper.find('.sidebar').classes()).toContain('expanded')

    await click('settings')
    expect(pane('main').isVisible()).toBe(true)
    expect(pane('settings').isVisible()).toBe(false)
    expect(button('settings').attributes('aria-pressed')).toBe('false')
    expect(wrapper.find('.sidebar').classes()).not.toContain('expanded')
  })

  it('shows Push-To alone and switches straight to Settings', async () => {
    await mountApp()

    await click('push-to')
    expect(pane('push-to').isVisible()).toBe(true)
    expect(pane('main').isVisible()).toBe(false)
    expect(button('push-to').attributes('aria-pressed')).toBe('true')

    await click('settings')
    expect(pane('settings').isVisible()).toBe(true)
    expect(pane('push-to').isVisible()).toBe(false)
    expect(pane('main').isVisible()).toBe(false)
    expect(button('push-to').attributes('aria-pressed')).toBe('false')
    expect(button('settings').attributes('aria-pressed')).toBe('true')
  })

  // Remounting a panel dropped what the observer had typed into it and refetched its
  // data; a hidden one also keeps its debounced saves alive.
  it('mounts each panel once, however often the tabs switch', async () => {
    await mountApp()

    for (const tab of ['settings', 'settings', 'push-to', 'settings', 'push-to', 'push-to']) {
      await click(tab)
    }

    expect(pane('main').isVisible()).toBe(true)
    expect(mounts).toEqual({ CameraPanel: 1, CaptureControls: 1, PushToPanel: 1, SettingsPanel: 1 })
  })

  it('opens the Push-To tab once the setup overlay has installed what was missing', async () => {
    const { appState } = await mountApp()
    const api = await import('./composables/api.js')
    appState.capabilities.value = { push_to: { astap_solver: true } }
    api.getAstapStatus.mockResolvedValueOnce({ ready: false })

    await click('push-to')
    expect(pane('main').isVisible()).toBe(true)
    expect(pane('push-to').exists()).toBe(false)

    await wrapper.find('[data-test="setup-installed"]').trigger('click')
    await flushPromises()
    expect(wrapper.find('[data-test="setup-installed"]').exists()).toBe(false)
    expect(pane('push-to').isVisible()).toBe(true)
    expect(pane('main').isVisible()).toBe(false)
  })
})
