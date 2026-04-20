import {describe, it, expect, vi, beforeEach} from 'vitest'
import {mount, flushPromises} from '@vue/test-utils'
import {ref} from 'vue'
import CameraPanel from './CameraPanel.vue'

// Mock the API module
vi.mock('../composables/api.js', () => ({
    connectCamera: vi.fn(),
    disconnectCamera: vi.fn(),
    configureSimulator: vi.fn(),
    getSimulatorConfig: vi.fn(),
}))

import {connectCamera, disconnectCamera, getSimulatorConfig} from '../composables/api.js'

describe('CameraPanel', () => {
    beforeEach(() => {
        vi.clearAllMocks()
        connectCamera.mockResolvedValue({message: 'Connected'})
        disconnectCamera.mockResolvedValue({message: 'Disconnected'})
        // Default: simulator is configured so the extra section doesn't show
        getSimulatorConfig.mockResolvedValue({
            configured: true,
            directory: '/some/path',
            file_count: 10,
        })
    })

    function createMockProvides(overrides = {}) {
        return {
            cameras: ref(overrides.cameras ?? []),
            selectedCamera: ref(overrides.selectedCamera ?? null),
            refreshCameras: vi.fn().mockResolvedValue(undefined),
            eventStream: {
                captureState: ref(overrides.captureState ?? 'Idle'),
                ...overrides.eventStream,
            },
            // Default: simulator disabled so it doesn't affect existing tests
            simulatorEnabled: ref(overrides.simulatorEnabled ?? false),
            cameraStatus: ref(overrides.cameraStatus ?? {}),
            cameraPhase: ref(overrides.cameraPhase ?? {}),
        }
    }

    function mountCameraPanel(provides = {}) {
        return mount(CameraPanel, {
            global: {
                provide: createMockProvides(provides),
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
            getSimulatorConfig.mockResolvedValue({configured: false, directory: null, file_count: null})
            const wrapper = mountCameraPanel({cameras: [], simulatorEnabled: true})
            await flushPromises()
            await wrapper.vm.$nextTick()

            // Should show simulator add button when enabled
            expect(wrapper.find('.simulator-add').exists()).toBe(true)
            expect(wrapper.text()).toContain('+ Add Simulated Camera')
        })

        it('shows camera count when simulators are configured', async () => {
            getSimulatorConfig.mockResolvedValue({
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
            const selectedCamera = ref(null)
            const wrapper = mount(CameraPanel, {
                global: {
                    provide: {
                        cameras: ref([
                            {
                                id: 'cam1',
                                name: 'Camera 1',
                                connected: true,
                                info: {max_width: 1920, max_height: 1080},
                            },
                        ]),
                        selectedCamera,
                        refreshCameras: vi.fn(),
                        eventStream: {captureState: ref('Idle')},
                        simulatorEnabled: ref(false),
                    },
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

            expect(connectCamera).toHaveBeenCalledWith('cam1')
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

        it('disables Disconnect button during capture', () => {
            const wrapper = mountCameraPanel({
                cameras: [
                    {
                        id: 'cam1',
                        name: 'Neptune-C II',
                        connected: true,
                        info: {max_width: 2712, max_height: 1538},
                    },
                ],
                captureState: 'Capturing',
            })

            expect(wrapper.find('.btn-danger').attributes('disabled')).toBeDefined()
        })

        it('shows ... while connecting', async () => {
            connectCamera.mockImplementation(() => new Promise(() => {
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
            connectCamera.mockRejectedValue(new Error('Connection failed'))

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
            expect(wrapper.find('.spinner').exists()).toBe(true)
            expect(wrapper.find('.btn-danger').text()).toContain('Warming up')
        })

        it('disables Disconnect while warming up', () => {
            const wrapper = mountCameraPanel({
                cameras: [cooledCamera],
                cameraPhase: {'Cooled Camera': 'warming_up'},
            })

            expect(wrapper.find('.btn-danger').attributes('disabled')).toBeDefined()
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
                    provide: {
                        cameras: ref([]),
                        selectedCamera: ref(null),
                        refreshCameras,
                        eventStream: {captureState: ref('Idle')},
                        simulatorEnabled: ref(false),
                    },
                },
            })

            await wrapper.find('.panel-header .btn').trigger('click')

            expect(refreshCameras).toHaveBeenCalled()
        })
    })
})
