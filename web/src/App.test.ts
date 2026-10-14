import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import type { VueWrapper } from '@vue/test-utils'
import { ref, h, onMounted } from 'vue'
import type { SetupContext } from 'vue'
import { aiComputeReport, astapStatus, capabilities, catalogStatus, settings } from './test/fixtures'
import type { EventStream } from './composables/useEventStream'

/** What the mocked `useAppState` hands App: what App, and the children left unstubbed, read. */
function mockAppState() {
  return {
    loading: ref(false),
    globalError: ref<string | null>(null),
    simulatorEnabled: ref(false),
    settings: ref(settings({ eula_accepted: true })),
    refreshSettings: vi.fn(),
    applySettings: vi.fn(),
    refreshCameras: vi.fn(),
    initializeState: vi.fn(),
    connectEvents: vi.fn(),
    cameras: ref([]),
    selection: ref(null),
    cameraStatus: ref(null),
    cameraPhase: ref(null),
    warmupEndsAt: ref({}),
    capabilities: ref(capabilities({ push_to: { astap_solver: false } })),
  }
}

/** Every app state App made, in order; a test's own is the last. */
const mockAppStates: ReturnType<typeof mockAppState>[] = []

// Mock composables
vi.mock('./composables/useAppState', () => ({
  useAppState: vi.fn(() => {
    const state = mockAppState()
    mockAppStates.push(state)
    return state
  }),
}))

vi.mock('./composables/useEventStream', async (importOriginal) => {
  const { createEventStreamStub } = await import('./test/eventStream')
  return { ...await importOriginal(), useEventStream: vi.fn(() => createEventStreamStub()) }
})

vi.mock('./composables/api', async (importOriginal) =>
  (await import('./test/mockApi')).mockApi(await importOriginal(), {
    getAstapStatus: vi.fn(async () => astapStatus({ ready: true })),
    getCatalogStatus: vi.fn(async () => catalogStatus({ installed: true })),
    getAiCompute: vi.fn(async () => aiComputeReport({ state: 'unavailable' })),
  }))

/** The app state App made last. */
function lastAppState() {
  const state = mockAppStates.at(-1)
  if (!state) throw new Error('App made no app state')
  return state
}

/** The event stream App opened last. */
async function lastEventStream(): Promise<EventStream> {
  const { useEventStream } = await import('./composables/useEventStream')
  const result = vi.mocked(useEventStream).mock.results.at(-1)
  if (result?.type !== 'return') throw new Error('App opened no event stream')
  return result.value
}

// We must mock App.vue import so that window.location is set BEFORE module is evaluated
// But we can also just use vi.doMock or isolateModules if needed.
// However, since we're using Vite/Vitest, we can manipulate window.location and dynamically import App.vue
describe('App.vue Routing', () => {
  /** App reads the route once, as its module is evaluated: before each `import('./App.vue')`. */
  const visit = (pathname: string) => vi.stubGlobal('location', { ...window.location, pathname })

  afterEach(() => {
    vi.resetModules()
  })

  it('renders EyepieceView on /eyepiece', async () => {
    visit('/eyepiece')
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
    visit('/eyepiece_quality')
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
    visit('/')
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
    visit('/')
    const api = await import('./composables/api')
    vi.mocked(api.getAiCompute).mockResolvedValueOnce(
      aiComputeReport({ state: 'benchmarking', progress: { done: 0, total: 2, testing: 'CPU — test' } }),
    )
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

  // The stores subscribe themselves; App only hands them the one stream.
  it('connects the app state and the AI compute report to the event stream', async () => {
    visit('/')
    const App = (await import('./App.vue')).default
    mount(App, { global: { stubs: { StatusBar: true, LiveView: true, CameraPanel: true, CaptureControls: true, SettingsPanel: true, PushToPanel: true } } })
    const appState = lastAppState()
    const events = await lastEventStream()

    expect(appState.connectEvents).toHaveBeenCalledExactlyOnceWith(events)
    expect(events.on).toHaveBeenCalledWith('ai_compute_changed', expect.any(Function))
    expect(events.onSync).toHaveBeenCalled()
  })

  it('leaves the app interactive when there is no benchmark', async () => {
    visit('/')
    const App = (await import('./App.vue')).default
    const wrapper = mount(App, { global: { stubs: { StatusBar: true, LiveView: true, CameraPanel: true, CaptureControls: true, SettingsPanel: true, PushToPanel: true } } })
    await flushPromises()
    expect(wrapper.find('.app').attributes('inert')).toBeUndefined()
    expect(wrapper.find('[data-test="benchmark-overlay"]').exists()).toBe(false)
  })
})

/** A stub panel that counts how often it mounts. */
function countingStub(name: string, mounts: Record<string, number>) {
  return {
    name,
    setup() {
      onMounted(() => { mounts[name] = (mounts[name] ?? 0) + 1 })
      return () => h('div', { class: `stub-${name}` })
    },
  }
}

describe('App.vue sidebar tabs', () => {
  let mounts: Record<string, number>
  let mountedApp: VueWrapper | undefined

  beforeEach(() => {
    mounts = {}
  })

  afterEach(() => {
    mountedApp?.unmount()
    vi.resetModules()
  })

  async function mountApp() {
    const App = (await import('./App.vue')).default
    const mounted = mount(App, {
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
            setup: (_: unknown, { emit }: SetupContext<['installed', 'close']>) =>
              () => h('button', { 'data-test': 'setup-installed', onClick: () => emit('installed') }),
          },
        },
      },
    })
    mountedApp = mounted
    await flushPromises()
    return { wrapper: mounted, appState: lastAppState() }
  }

  type Tab = 'main' | 'push-to' | 'settings'
  const app = () => {
    if (!mountedApp) throw new Error('mount the app first')
    return mountedApp
  }
  const pane = (name: Tab) => app().find(`[data-test="sidebar-${name}"]`)
  const button = (name: Exclude<Tab, 'main'>) => app().find(`[data-test="tab-${name}"]`)

  async function click(name: Exclude<Tab, 'main'>) {
    await button(name).trigger('click')
    await flushPromises()
  }

  it('shows only the camera and capture controls at first, mounting no other tab', async () => {
    const { wrapper } = await mountApp()

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
    const { wrapper } = await mountApp()

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

    for (const tab of ['settings', 'settings', 'push-to', 'settings', 'push-to', 'push-to'] as const) {
      await click(tab)
    }

    expect(pane('main').isVisible()).toBe(true)
    expect(mounts).toEqual({ CameraPanel: 1, CaptureControls: 1, PushToPanel: 1, SettingsPanel: 1 })
  })

  it('opens the Push-To tab once the setup overlay has installed what was missing', async () => {
    const { wrapper, appState } = await mountApp()
    const api = await import('./composables/api')
    appState.capabilities.value = capabilities({ push_to: { astap_solver: true } })
    vi.mocked(api.getAstapStatus).mockResolvedValueOnce(astapStatus({ ready: false }))

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
