import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {mount, flushPromises} from '@vue/test-utils'
import {ref, watch} from 'vue'
import CameraPanel from './CameraPanel.vue'

// Mock the API module
vi.mock('../composables/api', async (importOriginal) =>
    (await import('../test/mockApi')).mockApi(await importOriginal(), {
        connectCamera: vi.fn(),
        disconnectCamera: vi.fn(),
        configureSimulator: vi.fn(),
        getSimulatorConfig: vi.fn(),
        removeSimulatedCamera: vi.fn(),
    }))

import {
    configureSimulator,
    connectCamera,
    disconnectCamera,
    getSimulatorConfig,
    removeSimulatedCamera,
} from '../composables/api'
import {contextProvides} from '../composables/appContext'
import {createEventStreamStub} from '../test/eventStream'
import {cameraInfo, settings} from '../test/fixtures'
import type {Camera, CameraInfo, CameraPhase, CaptureState, Settings} from '../composables/api.types'
import type {ByCameraName, CameraStatus} from '../composables/cameraStore'

/** A camera list entry as a test writes it: the info it cares about. */
type CameraFields = Omit<Camera, 'info'> & {info?: Partial<CameraInfo>}

/** `fields` as the list hands them out, the info filled from the fixture. */
function listed({info, ...fields}: CameraFields): Camera {
    return {...fields, info: cameraInfo({id: fields.id, name: fields.name, ...info})}
}

/** The element `selector` finds in the page (the dialog and the menu teleport there); throws without one. */
function inBody<E extends HTMLElement = HTMLElement>(selector: string): E {
    const element = document.querySelector<E>(selector)
    if (!element) throw new Error(`nothing matches ${selector}`)
    return element
}

describe('CameraPanel', () => {
    beforeEach(() => {
        vi.clearAllMocks()
        // The Connect menu teleports to <body>; without this, one test's open menu is
        // still there for the next one's `document.querySelector` to find first.
        document.body.innerHTML = ''
        vi.mocked(connectCamera).mockResolvedValue({message: 'Connected'})
        vi.mocked(disconnectCamera).mockResolvedValue({message: 'Disconnected', camera_id: 'cam1', warming_up: false})
        // Default: simulator is configured so the extra section doesn't show
        vi.mocked(getSimulatorConfig).mockResolvedValue({
            configured: true,
            directory: '/some/path',
            file_count: 10,
        })
    })

    interface Overrides {
        cameras?: CameraFields[]
        selectedCamera?: string | null
        captureState?: CaptureState
        simulatorEnabled?: boolean
        cameraStatus?: ByCameraName<CameraStatus>
        cameraPhase?: ByCameraName<CameraPhase>
        warmupEndsAt?: ByCameraName<number>
        /** Over the default settings; none at all without it. */
        settings?: Partial<Settings>
    }

    function createMockProvides(overrides: Overrides = {}) {
        return {
            cameras: ref((overrides.cameras ?? []).map(listed)),
            selectedCamera: ref(overrides.selectedCamera ?? null),
            refreshCameras: vi.fn().mockResolvedValue(undefined),
            eventStream: createEventStreamStub({captureState: overrides.captureState ?? 'Idle'}),
            // Default: simulator disabled so it doesn't affect existing tests
            simulatorEnabled: ref(overrides.simulatorEnabled ?? false),
            cameraStatus: ref(overrides.cameraStatus ?? {}),
            cameraPhase: ref(overrides.cameraPhase ?? {}),
            warmupEndsAt: ref(overrides.warmupEndsAt ?? {}),
            settings: ref(overrides.settings ? settings(overrides.settings) : null),
        }
    }

    function mountCameraPanel(provides: Overrides = {}, options: {attachTo?: HTMLElement} = {}) {
        return mount(CameraPanel, {
            ...options,
            global: {
                provide: contextProvides(createMockProvides(provides)),
            },
        })
    }

    describe('Camera List', () => {
        it('shows empty state when no cameras found and simulator is configured', async () => {
            // Simulator is configured (default mock), so empty state should show
            const wrapper = mountCameraPanel({cameras: []})
            await flushPromises() // Wait for onMounted to complete

            expect(wrapper.find('.empty-state').exists()).toBe(true)
            expect(wrapper.text()).toContain('No cameras found')
        })

        it('shows simulator add section when simulator enabled', async () => {
            vi.mocked(getSimulatorConfig).mockResolvedValue({configured: false, directory: null, file_count: null})
            const wrapper = mountCameraPanel({cameras: [], simulatorEnabled: true})
            await flushPromises()
            await wrapper.vm.$nextTick()

            // Should show simulator add button when enabled
            expect(wrapper.find('.simulator-add').exists()).toBe(true)
            expect(wrapper.text()).toContain('+ Add Simulated Camera')
        })

        it('shows camera count when simulators are configured', async () => {
            vi.mocked(getSimulatorConfig).mockResolvedValue({
                configured: true,
                directory: '/some/path',
                file_count: 10,
                camera_count: 3,
            })
            const wrapper = mountCameraPanel({cameras: [], simulatorEnabled: true})
            await flushPromises()
            await wrapper.vm.$nextTick()

            // Should show simulator count
            expect(wrapper.find('.simulator-add').exists()).toBe(true)
            expect(wrapper.text()).toContain('3 configured')
        })

        it('hides simulator cameras when simulator toggle is disabled', async () => {
            const wrapper = mountCameraPanel({
                cameras: [
                    {
                        id: 'cam1',
                        name: 'Real Camera',
                        provider: 'PlayerOne',
                        connected: false,
                        info: {max_width: 1920, max_height: 1080},
                    },
                    {
                        id: 'sim1',
                        name: 'Simulator',
                        provider: 'Simulator',
                        connected: false,
                        info: {max_width: 1920, max_height: 1080},
                    },
                ],
                simulatorEnabled: false,
            })
            await flushPromises()

            // Only real camera should be shown
            const cameraNames = wrapper.findAll('.camera-name')
            expect(cameraNames.length).toBe(1)
            expect(cameraNames[0].text()).toBe('Real Camera')
        })

        it('shows simulator cameras when simulator toggle is enabled', async () => {
            const wrapper = mountCameraPanel({
                cameras: [
                    {
                        id: 'cam1',
                        name: 'Real Camera',
                        provider: 'PlayerOne',
                        connected: false,
                        info: {max_width: 1920, max_height: 1080},
                    },
                    {
                        id: 'sim1',
                        name: 'Simulator',
                        provider: 'Simulator',
                        connected: false,
                        info: {max_width: 1920, max_height: 1080},
                    },
                ],
                simulatorEnabled: true,
            })
            await flushPromises()

            // Both cameras should be shown
            const cameraNames = wrapper.findAll('.camera-name')
            expect(cameraNames.length).toBe(2)
        })

        it('shows connected cameras in connected section', () => {
            const wrapper = mountCameraPanel({
                cameras: [
                    {
                        id: 'cam1',
                        name: 'Neptune-C II',
                        connected: true,
                        info: {max_width: 2712, max_height: 1538},
                    },
                ],
            })

            expect(wrapper.find('.section-title').text()).toBe('Connected')
            expect(wrapper.find('.camera-name').text()).toBe('Neptune-C II')
        })

        it('shows available cameras in available section', () => {
            const wrapper = mountCameraPanel({
                cameras: [
                    {
                        id: 'cam1',
                        name: 'Mars-M',
                        connected: false,
                        info: {max_width: 1920, max_height: 1080},
                    },
                ],
            })

            expect(wrapper.find('.section-title').text()).toBe('Available')
            expect(wrapper.find('.camera-name').text()).toBe('Mars-M')
        })

        it('shows both sections when both connected and available cameras exist', async () => {
            const wrapper = mountCameraPanel({
                cameras: [
                    {
                        id: 'cam1',
                        name: 'Neptune-C II',
                        connected: true,
                        info: {max_width: 2712, max_height: 1538},
                    },
                    {
                        id: 'cam2',
                        name: 'Mars-M',
                        connected: false,
                        info: {max_width: 1920, max_height: 1080},
                    },
                ],
            })
            await flushPromises() // Wait for onMounted to complete

            const sections = wrapper.findAll('.section-title')
            expect(sections.length).toBe(2)
            expect(sections[0].text()).toBe('Connected')
            expect(sections[1].text()).toBe('Available')
        })

        it('displays camera resolution', () => {
            const wrapper = mountCameraPanel({
                cameras: [
                    {
                        id: 'cam1',
                        name: 'Neptune-C II',
                        connected: true,
                        info: {max_width: 2712, max_height: 1538},
                    },
                ],
            })

            expect(wrapper.find('.camera-details').text()).toBe('2712x1538')
        })

        it('shows an unknown resolution as a dash, not 0x0', () => {
            const wrapper = mountCameraPanel({
                cameras: [
                    {
                        id: 'qhy_sn-QHY268M-1a2b',
                        name: 'QHY268M-1a2b',
                        connected: false,
                        info: {max_width: 0, max_height: 0},
                    },
                ],
            })

            expect(wrapper.find('.camera-details').text()).toBe('—')
        })
    })

    describe('Camera Selection', () => {
        it('highlights selected camera', () => {
            const wrapper = mountCameraPanel({
                cameras: [
                    {
                        id: 'cam1',
                        name: 'Camera 1',
                        connected: true,
                        info: {max_width: 1920, max_height: 1080},
                    },
                    {
                        id: 'cam2',
                        name: 'Camera 2',
                        connected: true,
                        info: {max_width: 1920, max_height: 1080},
                    },
                ],
                selectedCamera: 'cam1',
            })

            const items = wrapper.findAll('.camera-item')
            expect(items[0].classes()).toContain('selected')
            expect(items[1].classes()).not.toContain('selected')
        })

        it('selects camera when clicking on it', async () => {
            const selectedCamera = ref<string | null>(null)
            const wrapper = mount(CameraPanel, {
                global: {
                    provide: contextProvides({
                        cameras: ref([
                            listed({
                                id: 'cam1',
                                name: 'Camera 1',
                                connected: true,
                                info: {max_width: 1920, max_height: 1080},
                            }),
                        ]),
                        selectedCamera,
                        refreshCameras: vi.fn(),
                        eventStream: createEventStreamStub({captureState: 'Idle'}),
                        simulatorEnabled: ref(false),
                    }),
                },
            })

            await wrapper.find('.camera-item').trigger('click')

            expect(selectedCamera.value).toBe('cam1')
        })
    })

    describe('Connect/Disconnect', () => {
        it('shows Connect button for available cameras', () => {
            const wrapper = mountCameraPanel({
                cameras: [
                    {
                        id: 'cam1',
                        name: 'Mars-M',
                        connected: false,
                        info: {max_width: 1920, max_height: 1080},
                    },
                ],
            })

            expect(wrapper.find('.btn-primary').text()).toBe('Connect')
        })

        it('shows Disconnect button for connected cameras', () => {
            const wrapper = mountCameraPanel({
                cameras: [
                    {
                        id: 'cam1',
                        name: 'Neptune-C II',
                        connected: true,
                        info: {max_width: 2712, max_height: 1538},
                    },
                ],
            })

            expect(wrapper.find('.btn-danger').text()).toBe('Disconnect')
        })

        it('calls connectCamera when Connect button clicked', async () => {
            const wrapper = mountCameraPanel({
                cameras: [
                    {
                        id: 'cam1',
                        name: 'Mars-M',
                        connected: false,
                        info: {max_width: 1920, max_height: 1080},
                    },
                ],
            })

            await wrapper.find('.btn-primary').trigger('click')
            await flushPromises()

            expect(connectCamera).toHaveBeenCalledWith('cam1', 'main')
        })

        // The connect stored the camera's profiles; Push-To resolves the selected camera
        // from them, and without them derived its optics from the previous camera's.
        it('loads the settings before it selects the connected camera', async () => {
            let settingsLoaded = false
            let loadedWhenSelected = null
            const provide = {
                ...createMockProvides({
                    cameras: [{id: 'cam1', name: 'Mars-M', connected: false, info: {}}],
                }),
                refreshSettings: vi.fn(async () => {
                    await Promise.resolve()
                    settingsLoaded = true
                    return null
                }),
            }
            watch(provide.selectedCamera, () => { loadedWhenSelected = settingsLoaded }, {flush: 'sync'})
            const wrapper = mount(CameraPanel, {global: {provide: contextProvides(provide)}})

            await wrapper.find('.btn-primary').trigger('click')
            await flushPromises()

            expect(provide.selectedCamera.value).toBe('cam1')
            expect(loadedWhenSelected).toBe(true)
        })

        // Review of 4fc5053, R4: with nothing selected, refreshing the list selects the
        // camera itself (useAppState), so it must not run before the settings land.
        it('loads the settings before the list refresh selects a first camera', async () => {
            let settingsLoaded = false
            let loadedWhenSelected = null
            const base = createMockProvides({
                cameras: [{id: 'cam1', name: 'Mars-M', connected: false, info: {}}],
            })
            const provide = {
                ...base,
                refreshCameras: vi.fn(async () => {
                    if (!base.selectedCamera.value) base.selectedCamera.value = 'cam1'
                    return base.cameras.value
                }),
                refreshSettings: vi.fn(async () => {
                    await new Promise((resolve) => setTimeout(resolve, 0))
                    settingsLoaded = true
                    return null
                }),
            }
            watch(provide.selectedCamera, () => { loadedWhenSelected = settingsLoaded }, {flush: 'sync'})
            const wrapper = mount(CameraPanel, {global: {provide: contextProvides(provide)}})

            await wrapper.find('.btn-primary').trigger('click')
            await new Promise((resolve) => setTimeout(resolve, 5))
            await flushPromises()

            expect(provide.selectedCamera.value).toBe('cam1')
            expect(loadedWhenSelected).toBe(true)
        })

        it('calls disconnectCamera when Disconnect button clicked', async () => {
            const wrapper = mountCameraPanel({
                cameras: [
                    {
                        id: 'cam1',
                        name: 'Neptune-C II',
                        connected: true,
                        info: {max_width: 2712, max_height: 1538},
                    },
                ],
            })

            await wrapper.find('.btn-danger').trigger('click')
            await flushPromises()

            expect(disconnectCamera).toHaveBeenCalledWith('cam1')
        })

        // Disconnect is a must (2026-09-20): during a capture it stops the capture first,
        // which is worth one confirmation, not a greyed-out button.
        it('asks before stopping a capture to disconnect the imaging camera', async () => {
            const wrapper = mountCameraPanel({
                cameras: [
                    {
                        id: 'cam1',
                        name: 'Ares-C PRO',
                        role: 'main',
                        connected: true,
                        info: {max_width: 3008, max_height: 3008},
                    },
                ],
                captureState: 'Capturing',
            })

            const button = wrapper.find('.btn-danger')
            expect(button.attributes('disabled')).toBeUndefined()
            await button.trigger('click')
            await flushPromises()
            expect(disconnectCamera).not.toHaveBeenCalled()
            expect(document.body.textContent).toContain('Stop the capture and disconnect?')

            inBody('.confirm-disconnect').click()
            await flushPromises()
            expect(disconnectCamera).toHaveBeenCalledWith('cam1')
        })

        it('leaves the camera connected when the confirmation is cancelled', async () => {
            const wrapper = mountCameraPanel({
                cameras: [{id: 'cam1', name: 'Ares-C PRO', role: 'main', connected: true, info: {}}],
                captureState: 'Recovering',
            })

            await wrapper.find('.btn-danger').trigger('click')
            await flushPromises()
            inBody('.confirm-cancel').click()
            await flushPromises()

            expect(disconnectCamera).not.toHaveBeenCalled()
            expect(document.querySelector('.confirm-disconnect')).toBeNull()
        })

        // The warm-up can finish while the dialog is open; confirming then answered
        // "not connected" for a camera that had already gone the way it was asked to.
        it('closes the warm-up confirmation once the camera has finished warming up', async () => {
            const provides = createMockProvides({
                cameras: [{id: 'cam1', name: 'Ares-C PRO', role: 'main', connected: true, info: {}}],
                cameraPhase: {'Ares-C PRO': 'warming_up'},
            })
            const wrapper = mount(CameraPanel, {global: {provide: contextProvides(provides)}})

            await wrapper.find('.btn-danger').trigger('click')
            await flushPromises()
            expect(document.body.textContent).toContain('Disconnect without warming up?')

            provides.cameraPhase.value = {}
            provides.cameras.value = [listed({id: 'cam1', name: 'Ares-C PRO', connected: false})]
            await flushPromises()

            expect(document.querySelector('.confirm-disconnect')).toBeNull()
            expect(disconnectCamera).not.toHaveBeenCalled()
        })

        it('closes the capture confirmation once the capture has ended', async () => {
            const provides = createMockProvides({
                cameras: [{id: 'cam1', name: 'Ares-C PRO', role: 'main', connected: true, info: {}}],
                captureState: 'Capturing',
            })
            const wrapper = mount(CameraPanel, {global: {provide: contextProvides(provides)}})

            await wrapper.find('.btn-danger').trigger('click')
            await flushPromises()
            expect(document.body.textContent).toContain('The sub being exposed is discarded')

            provides.eventStream.captureState.value = 'Idle'
            await flushPromises()

            expect(document.querySelector('.confirm-disconnect')).toBeNull()
            expect(disconnectCamera).not.toHaveBeenCalled()
        })

        // The guide camera has no tie to the capture; the 14:59 guide unplugged mid-capture
        // could not be disconnected for minutes because this button was greyed out.
        it('disconnects the guide camera during a main capture without asking', async () => {
            const wrapper = mountCameraPanel({
                cameras: [
                    {
                        id: 'guide1',
                        name: 'Neptune-C II',
                        role: 'guide',
                        connected: true,
                        info: {max_width: 2712, max_height: 1538},
                    },
                ],
                captureState: 'Capturing',
            })

            await wrapper.find('.btn-danger').trigger('click')
            await flushPromises()

            expect(disconnectCamera).toHaveBeenCalledWith('guide1')
        })

        it('shows ... while connecting', async () => {
            vi.mocked(connectCamera).mockImplementation(() => new Promise(() => {
            })) // Never resolves

            const wrapper = mountCameraPanel({
                cameras: [
                    {
                        id: 'cam1',
                        name: 'Mars-M',
                        connected: false,
                        info: {max_width: 1920, max_height: 1080},
                    },
                ],
            })

            await wrapper.find('.btn-primary').trigger('click')
            await flushPromises()

            // The component shows '...' while connecting
            expect(wrapper.find('.btn-primary').text()).toBe('...')
        })

        it('shows error message when connect fails', async () => {
            vi.mocked(connectCamera).mockRejectedValue(new Error('Connection failed'))

            const wrapper = mountCameraPanel({
                cameras: [
                    {
                        id: 'cam1',
                        name: 'Mars-M',
                        connected: false,
                        info: {max_width: 1920, max_height: 1080},
                    },
                ],
            })

            await wrapper.find('.btn-primary').trigger('click')
            await flushPromises()

            expect(wrapper.find('.alert-error').text()).toContain('Connection failed')
        })
    })

    describe('Camera roles', () => {
        const availableCamera = {
            id: 'cam1',
            name: 'Mars-M',
            connected: false,
            info: {max_width: 1920, max_height: 1080},
        }

        it('offers the guide slot behind the Connect chevron', async () => {
            const wrapper = mountCameraPanel({cameras: [availableCamera]})

            expect(wrapper.find('.split-button-menu').exists()).toBe(false)

            await wrapper.find('.split-button-toggle').trigger('click')

            const items = document.querySelectorAll('.split-button-item')
            expect(items.length).toBe(1)
            expect(items[0].textContent.trim()).toBe('As guide')
        })

        it('connects as a guide camera when that option is chosen', async () => {
            const wrapper = mountCameraPanel({cameras: [availableCamera]})

            await wrapper.find('.split-button-toggle').trigger('click')
            inBody('.split-button-item').click()
            await flushPromises()

            expect(connectCamera).toHaveBeenCalledWith('cam1', 'guide')
        })

        it('disables the guide option once a guide camera is connected', async () => {
            const wrapper = mountCameraPanel({
                cameras: [
                    availableCamera,
                    {
                        id: 'cam2',
                        name: 'Guider',
                        connected: true,
                        role: 'guide',
                        info: {max_width: 640, max_height: 480},
                    },
                ],
            })

            await wrapper.find('.split-button-toggle').trigger('click')

            expect(inBody<HTMLButtonElement>('.split-button-item').disabled).toBe(true)
        })

        it('labels each connected camera with the position it holds', () => {
            const wrapper = mountCameraPanel({
                cameras: [
                    {
                        id: 'cam1',
                        name: 'Imaging',
                        connected: true,
                        role: 'main',
                        info: {max_width: 1920, max_height: 1080},
                    },
                    {
                        id: 'cam2',
                        name: 'Guider',
                        connected: true,
                        role: 'guide',
                        info: {max_width: 640, max_height: 480},
                    },
                ],
            })

            const pills = wrapper.findAll('.role-pill').map((p) => p.text())
            expect(pills).toEqual(['Main', 'Guide'])
        })
    })

    describe('Lifecycle phase display', () => {
        const cooledCamera = {
            id: 'cam1',
            name: 'Cooled Camera',
            connected: true,
            info: {max_width: 1920, max_height: 1080, has_cooler: true},
        }

        it('shows Precooling pill when phase is precooling', () => {
            const wrapper = mountCameraPanel({
                cameras: [cooledCamera],
                cameraPhase: {'Cooled Camera': 'precooling'},
            })

            const pill = wrapper.find('.phase-pill')
            expect(pill.exists()).toBe(true)
            expect(pill.text()).toBe('Precooling')
        })

        it('shows Warming up pill and spinner when phase is warming_up', () => {
            const wrapper = mountCameraPanel({
                cameras: [cooledCamera],
                cameraPhase: {'Cooled Camera': 'warming_up'},
            })

            expect(wrapper.find('.phase-pill').text()).toBe('Warming up')
            expect(wrapper.find('.phase-pill .base-spinner').exists()).toBe(true)
            expect(wrapper.find('.btn-danger').text()).toBe('Disconnect now')
        })

        it('says how long the warm-up may still take', () => {
            const wrapper = mountCameraPanel({
                cameras: [cooledCamera],
                cameraPhase: {'Cooled Camera': 'warming_up'},
                warmupEndsAt: {'Cooled Camera': Date.now() + 4.5 * 60_000},
            })

            expect(wrapper.find('.phase-pill').text()).toBe('Warming up, up to 5 min')
        })

        it('skips the warm-up only after the user confirms it', async () => {
            const wrapper = mountCameraPanel({
                cameras: [cooledCamera],
                cameraPhase: {'Cooled Camera': 'warming_up'},
            })

            const button = wrapper.find('.btn-danger')
            expect(button.attributes('disabled')).toBeUndefined()
            await button.trigger('click')
            await flushPromises()
            expect(disconnectCamera).not.toHaveBeenCalled()
            expect(document.body.textContent).toContain('Disconnect without warming up?')

            inBody('.confirm-disconnect').click()
            await flushPromises()
            expect(disconnectCamera).toHaveBeenCalledWith('cam1', {skipWarmup: true})
        })

        // The server reopens a dropped camera without disconnecting it; the panel must
        // not advertise a hiccup the reconnect hides.
        it('shows the camera as connected with no pill while it is recovering', () => {
            const wrapper = mountCameraPanel({
                cameras: [cooledCamera],
                cameraPhase: {'Cooled Camera': 'recovering'},
            })

            expect(wrapper.find('.phase-pill').exists()).toBe(false)
            expect(wrapper.find('.btn-danger').text()).toBe('Disconnect')
        })

        it('omits phase pill when camera is idle', () => {
            const wrapper = mountCameraPanel({
                cameras: [cooledCamera],
                cameraPhase: {'Cooled Camera': 'idle'},
            })

            expect(wrapper.find('.phase-pill').exists()).toBe(false)
            expect(wrapper.find('.btn-danger').text()).toBe('Disconnect')
        })
    })

    describe('Sensor mode pill display', () => {
        // Mirrors the real Ares-C PRO's advertised modes (see
        // camera/playerone/sensor_mode.rs's resolve_mode_index tests).
        const dualSamplingCamera = {
            id: 'cam1',
            name: 'Ares-C PRO',
            connected: true,
            info: {
                max_width: 3008,
                max_height: 3008,
                sensor_modes: [
                    {index: 0, name: 'Normal', description: ''},
                    {index: 1, name: 'Low Noise', description: ''},
                ],
            },
        }

        it('shows Low Noise for Deep Sky while actively stacking', () => {
            const wrapper = mountCameraPanel({
                cameras: [dualSamplingCamera],
                settings: {stacking: true, stacking_type: 'deep_sky', sensor_mode_override: null},
            })

            expect(wrapper.find('.sensor-mode-pill').text()).toBe('Low Noise')
        })

        it('shows Normal for Deep Sky when not stacking (live view)', () => {
            const wrapper = mountCameraPanel({
                cameras: [dualSamplingCamera],
                settings: {stacking: false, stacking_type: 'deep_sky', sensor_mode_override: null},
            })

            expect(wrapper.find('.sensor-mode-pill').text()).toBe('Normal')
        })

        it('shows Low Noise for Comet while actively stacking', () => {
            const wrapper = mountCameraPanel({
                cameras: [dualSamplingCamera],
                settings: {stacking: true, stacking_type: 'comet', sensor_mode_override: null},
            })

            expect(wrapper.find('.sensor-mode-pill').text()).toBe('Low Noise')
        })

        it('shows Normal for Planetary regardless of stacking', () => {
            const wrapper = mountCameraPanel({
                cameras: [dualSamplingCamera],
                settings: {stacking: true, stacking_type: 'planetary', sensor_mode_override: null},
            })

            expect(wrapper.find('.sensor-mode-pill').text()).toBe('Normal')
        })

        it('sensor_mode_override wins regardless of stacking state', () => {
            const wrapper = mountCameraPanel({
                cameras: [dualSamplingCamera],
                settings: {stacking: false, stacking_type: 'deep_sky', sensor_mode_override: 'low_readout_noise'},
            })

            expect(wrapper.find('.sensor-mode-pill').text()).toBe('Low Noise')
        })

        it('omits the pill when the camera advertises no sensor modes', () => {
            const wrapper = mountCameraPanel({
                cameras: [{...dualSamplingCamera, info: {max_width: 1920, max_height: 1080, sensor_modes: []}}],
                settings: {stacking: true, stacking_type: 'deep_sky', sensor_mode_override: null},
            })

            expect(wrapper.find('.sensor-mode-pill').exists()).toBe(false)
        })
    })

    describe('Camera Selection Display', () => {
        it('highlights selected camera in list', () => {
            const wrapper = mountCameraPanel({
                cameras: [
                    {
                        id: 'cam1',
                        name: 'Neptune-C II',
                        connected: true,
                        info: {
                            max_width: 2712,
                            max_height: 1538,
                        },
                    },
                ],
                selectedCamera: 'cam1',
            })

            const selectedItem = wrapper.find('.camera-item.selected')
            expect(selectedItem.exists()).toBe(true)
            expect(selectedItem.find('.camera-name').text()).toBe('Neptune-C II')
            expect(selectedItem.find('.camera-details').text()).toBe('2712x1538')
        })

        it('does not highlight any camera when none selected', () => {
            const wrapper = mountCameraPanel({
                cameras: [
                    {
                        id: 'cam1',
                        name: 'Camera',
                        connected: true,
                        info: {max_width: 1920, max_height: 1080},
                    },
                ],
                selectedCamera: null,
            })

            expect(wrapper.find('.camera-item.selected').exists()).toBe(false)
        })
    })

    describe('Refresh', () => {
        it('calls refreshCameras when refresh button clicked', async () => {
            const refreshCameras = vi.fn().mockResolvedValue(undefined)
            const wrapper = mount(CameraPanel, {
                global: {
                    provide: contextProvides({
                        cameras: ref([]),
                        selectedCamera: ref(null),
                        refreshCameras,
                        eventStream: createEventStreamStub({captureState: 'Idle'}),
                        simulatorEnabled: ref(false),
                    }),
                },
            })

            await wrapper.find('.panel-header .btn').trigger('click')

            // Not the click: the list refresh takes options, which a MouseEvent is not.
            expect(refreshCameras).toHaveBeenCalledWith()
        })
    })

    describe('Simulator', () => {
        const SIM = listed({id: 'sim_0', name: 'Simulator 1', provider: 'Simulator', index: 0, connected: true, info: {max_width: 100, max_height: 100}})
        const REAL = listed({id: 'zwo_0', name: 'ZWO ASI533', provider: 'ZWO', connected: true, info: {max_width: 3008, max_height: 3008}})

        it('adds a simulated camera from a directory, then lists it', async () => {
            vi.mocked(configureSimulator).mockResolvedValue({configured: true, camera_count: 1})
            const provides = createMockProvides({simulatorEnabled: true})
            const wrapper = mount(CameraPanel, {global: {provide: contextProvides(provides)}})
            await flushPromises()

            await wrapper.find('.simulator-add .btn').trigger('click')
            await wrapper.find('.directory-input').setValue('  /data/m42  ')
            await wrapper.find('.config-body .btn-primary').trigger('click')
            await flushPromises()

            expect(configureSimulator).toHaveBeenCalledWith('/data/m42')
            expect(provides.refreshCameras).toHaveBeenCalled()
            expect(wrapper.find('.directory-input').exists()).toBe(false)
            expect(wrapper.find('.simulator-count').text()).toBe('1 configured')
        })

        it('asks for a path instead of sending an empty one', async () => {
            const wrapper = mountCameraPanel({simulatorEnabled: true})
            await flushPromises()

            await wrapper.find('.simulator-add .btn').trigger('click')
            await wrapper.find('.config-body .btn-primary').trigger('click')

            expect(configureSimulator).not.toHaveBeenCalled()
            expect(wrapper.find('.alert-error').text()).toContain('Please enter a directory path')
        })

        it('removes a simulated camera, and moves the selection off it', async () => {
            vi.mocked(removeSimulatedCamera).mockResolvedValue({configured: true})
            const provides = createMockProvides({simulatorEnabled: true, cameras: [SIM, REAL], selectedCamera: 'sim_0'})
            provides.refreshCameras.mockImplementation(async () => {
                provides.cameras.value = [REAL]
            })
            const wrapper = mount(CameraPanel, {global: {provide: contextProvides(provides)}})
            await flushPromises()

            await wrapper.find('.btn-icon').trigger('click')
            await flushPromises()

            expect(removeSimulatedCamera).toHaveBeenCalledWith(0)
            expect(provides.selectedCamera.value).toBe('zwo_0')
            expect(getSimulatorConfig).toHaveBeenCalledTimes(2)
        })

        it('offers no removal while a capture runs on a connected one', async () => {
            const wrapper = mountCameraPanel({simulatorEnabled: true, cameras: [SIM], captureState: 'Capturing'})
            await flushPromises()

            expect(wrapper.find<HTMLButtonElement>('.btn-icon').element.disabled).toBe(true)
        })

        // `DELETE /api/simulator/{index}` names the camera by its index alone.
        it('removes nothing for a simulated camera listed without an index', async () => {
            const provides = createMockProvides({simulatorEnabled: true, cameras: [{...SIM, index: null}]})
            const wrapper = mount(CameraPanel, {global: {provide: contextProvides(provides)}})
            await flushPromises()

            await wrapper.find('.btn-icon').trigger('click')
            await flushPromises()

            expect(removeSimulatedCamera).not.toHaveBeenCalled()
        })
    })

    // As `useCaptureTarget` has it: with no stream to say otherwise, nothing is capturing.
    it('reads the imaging camera as idle without a provided event stream', async () => {
        const {eventStream: _stream, ...provides} = createMockProvides({
            cameras: [{id: 'cam1', name: 'Ares-C PRO', role: 'main', connected: true}],
        })
        const wrapper = mount(CameraPanel, {global: {provide: contextProvides(provides)}})
        await flushPromises()

        await wrapper.find('.btn-danger').trigger('click')
        await flushPromises()

        expect(document.querySelector('.confirm-disconnect')).toBeNull()
        expect(disconnectCamera).toHaveBeenCalledWith('cam1')
    })

    it('names the selected camera while the list is folded', async () => {
        const camera = {id: 'zwo_0', name: 'ZWO ASI533', provider: 'ZWO', connected: true, info: {max_width: 3008, max_height: 3008}}
        const wrapper = mountCameraPanel({cameras: [camera], selectedCamera: 'zwo_0'})

        await wrapper.find('.collapse-toggle').trigger('click')

        expect(wrapper.find('.cameras-container').attributes('style')).toContain('display: none')
        expect(wrapper.find('.collapsed-summary').text()).toContain('ZWO ASI533')
        expect(wrapper.find('.collapsed-summary').text()).toContain('3008x3008')
    })

    describe('folding with the Connect menu open', () => {
        let wrapper: ReturnType<typeof mountCameraPanel>

        beforeEach(async () => {
            const host = document.body.appendChild(document.createElement('div'))
            // Attached, so a click bubbles to `document` as in the browser.
            wrapper = mountCameraPanel(
                {cameras: [{id: 'zwo_0', name: 'ZWO ASI533MC', provider: 'Zwo', connected: false, info: {}}]},
                {attachTo: host},
            )
            await flushPromises()
            await wrapper.find('.split-button-toggle').trigger('click')
            expect(document.querySelector('.split-button-menu')).not.toBeNull()
        })

        afterEach(() => wrapper.unmount())

        it('closes it on a click on another header button', async () => {
            wrapper.find<HTMLButtonElement>('button[title="Refresh"]').element.click()
            await flushPromises()
            expect(document.querySelector('.split-button-menu')).toBeNull()
        })

        // The teleported menu once stayed floating, still live, over the folded panel.
        it('closes it on a click on the fold toggle', async () => {
            wrapper.find<HTMLButtonElement>('.collapse-toggle').element.click()
            await flushPromises()
            expect(document.querySelector('.split-button-menu')).toBeNull()
        })
    })
})
