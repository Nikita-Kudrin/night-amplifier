import {mount, flushPromises} from '@vue/test-utils'
import {ref} from 'vue'
import CaptureControls from './CaptureControls.vue'

// Mock the API module
vi.mock('../composables/api.js', () => ({
    startCapture: vi.fn(),
    stopCapture: vi.fn(),
    updateSettings: vi.fn(),
    getStackingTypes: vi.fn(),
}))

import {startCapture, stopCapture, updateSettings, getStackingTypes} from '../composables/api.js'

describe('CaptureControls', () => {
    beforeEach(() => {
        vi.clearAllMocks()
        // Default mock implementations
        startCapture.mockResolvedValue({message: 'Started'})
        stopCapture.mockResolvedValue({message: 'Stopped'})
        updateSettings.mockResolvedValue({})
        getStackingTypes.mockResolvedValue([
            {id: 'deep_sky', name: 'Deep Sky'},
            {id: 'planetary', name: 'Planetary'},
        ])
    })

    function createMockProvides(overrides = {}) {
        return {
            settings: ref({
                exposure_us: 1000000,
                gain: 100,
                offset: 10,
                bin: 1,
                auto_stretch: true,
                stacking: true,
                rejection_sigma: 2.5,
                background_subtraction: true,
                ...overrides.settings,
            }),
            selectedCamera: ref('selectedCamera' in overrides ? overrides.selectedCamera : 'cam1'),
            // Which camera the panel addresses. Start/Stop follows it, so a 'guide'
            // selection drives the guide loop rather than a capture session.
            selectedCameraRole: ref(overrides.selectedCameraRole ?? 'main'),
            mainCamera: ref(
                'mainCamera' in overrides ? overrides.mainCamera : {id: 'cam1', role: 'main'},
            ),
            guideCamera: ref('guideCamera' in overrides ? overrides.guideCamera : null),
            // Keyed by camera *name*: 'guiding' is the phase the backend stamps while
            // the guide loop owns the handle.
            cameraPhase: ref(overrides.cameraPhase ?? {}),
            eventStream: {
                captureState: ref(overrides.captureState ?? 'Idle'),
                ...overrides.eventStream,
            },
            refreshSettings: vi.fn().mockResolvedValue(undefined),
            capabilities: ref(overrides.capabilities ?? {
                has_pro: false,
                deep_sky: {advanced_rejection: false, rbf_background: false},
                planetary: {advanced_stacking: false},
                push_to: {astap_solver: false},
            }),
        }
    }

    function mountCaptureControls(provides = {}) {
        return mount(CaptureControls, {
            global: {
                provide: createMockProvides(provides),
                stubs: {
                    BaseProLock: true,
                },
            },
        })
    }

    describe('Start/Stop Button', () => {
        it('shows Start button when not capturing', () => {
            const wrapper = mountCaptureControls()

            const button = wrapper.find('.btn-capture')
            expect(button.classes()).toContain('btn-start')
            expect(button.text()).toContain('Start')
        })

        it('shows Stop button when capturing', () => {
            const wrapper = mountCaptureControls({
                captureState: 'Capturing',
            })

            const button = wrapper.find('.btn-capture')
            expect(button.classes()).toContain('btn-stop')
            expect(button.text()).toContain('Stop')
        })

        it('disables Start button when no camera selected', () => {
            const wrapper = mountCaptureControls({
                selectedCamera: null,
            })

            const button = wrapper.find('.btn-capture')
            // Vue renders disabled="" for truthy disabled attribute
            expect(button.element.disabled).toBe(true)
        })

        it('disables Stop button when stopping', () => {
            // When stopping, the button shows "Stopping..." but it's still the Start button
            // because isCapturing is false (Stopping != Capturing/Starting)
            const wrapper = mountCaptureControls({
                captureState: 'Stopping',
            })

            const button = wrapper.find('.btn-capture')
            // Button should be disabled when stopping
            expect(button.element.disabled).toBe(true)
        })

        it('calls startCapture when clicking Start', async () => {
            const wrapper = mountCaptureControls()

            await wrapper.find('.btn-start').trigger('click')
            await flushPromises()

            expect(updateSettings).toHaveBeenCalled()
            expect(startCapture).toHaveBeenCalledWith('cam1')
        })

        it('sends no camera at all when there is no imaging camera to capture with', async () => {
            // Falling back to the selected camera sent whatever id happened to be
            // selected, and with only a guide camera in the rig the server answered
            // with a role-mismatch message about connecting — a remedy for a request
            // the user had not made. Sending nothing lets the server say plainly that
            // an imaging camera is missing.
            const wrapper = mountCaptureControls({
                mainCamera: null,
                selectedCamera: 'stale-cam',
            })

            await wrapper.find('.btn-start').trigger('click')
            await flushPromises()

            expect(startCapture).toHaveBeenCalledWith(null)
        })

        it('captures on the imaging camera while the imaging camera is selected', async () => {
            const wrapper = mountCaptureControls({
                mainCamera: {id: 'imaging-cam', role: 'main'},
                selectedCamera: 'imaging-cam',
            })

            await wrapper.find('.btn-start').trigger('click')
            await flushPromises()

            expect(startCapture).toHaveBeenCalledWith('imaging-cam')
        })

        it('calls stopCapture when clicking Stop', async () => {
            const wrapper = mountCaptureControls({
                captureState: 'Capturing',
            })

            await wrapper.find('.btn-stop').trigger('click')
            await flushPromises()

            expect(stopCapture).toHaveBeenCalled()
        })

        it('calls stopCapture for the imaging camera by name', async () => {
            const wrapper = mountCaptureControls({captureState: 'Capturing'})

            await wrapper.find('.btn-stop').trigger('click')
            await flushPromises()

            expect(stopCapture).toHaveBeenCalledWith('main')
        })

        it('shows error message when start fails', async () => {
            startCapture.mockRejectedValue(new Error('Camera not ready'))

            const wrapper = mountCaptureControls()

            await wrapper.find('.btn-start').trigger('click')
            await flushPromises()

            expect(wrapper.find('.alert-error').text()).toContain('Camera not ready')
        })
    })

    // The guide camera is started and stopped from the same button, because it is the
    // same question: what should the camera I have selected be doing? Stopping it is
    // the only way to stop the raw frames it saves without disconnecting it.
    describe('Start/Stop with the guide camera selected', () => {
        const GUIDE = {id: 'guide-cam', name: 'Guide Cam', role: 'guide'}

        function mountGuideSelected(overrides = {}) {
            return mountCaptureControls({
                selectedCameraRole: 'guide',
                selectedCamera: GUIDE.id,
                guideCamera: GUIDE,
                mainCamera: {id: 'imaging-cam', role: 'main'},
                ...overrides,
            })
        }

        it('offers Start while the guide loop is not running', () => {
            const wrapper = mountGuideSelected({cameraPhase: {[GUIDE.name]: 'idle'}})

            const button = wrapper.find('.btn-capture')
            expect(button.classes()).toContain('btn-start')
            expect(button.text()).toContain('Start guide')
            expect(button.element.disabled).toBe(false)
        })

        it('offers Stop while the guide loop is running', () => {
            const wrapper = mountGuideSelected({cameraPhase: {[GUIDE.name]: 'guiding'}})

            const button = wrapper.find('.btn-capture')
            expect(button.classes()).toContain('btn-stop')
            expect(button.text()).toContain('Stop guide')
        })

        // The guide loop is not a capture session, so the imaging camera's state must
        // not decide what this button says.
        it('ignores the imaging camera capture state', () => {
            const wrapper = mountGuideSelected({
                captureState: 'Capturing',
                cameraPhase: {[GUIDE.name]: 'idle'},
            })

            expect(wrapper.find('.btn-capture').classes()).toContain('btn-start')
        })

        it('starts the guide camera without naming it', async () => {
            const wrapper = mountGuideSelected({cameraPhase: {[GUIDE.name]: 'idle'}})

            await wrapper.find('.btn-start').trigger('click')
            await flushPromises()

            expect(startCapture).toHaveBeenCalledWith(null, 'guide')
        })

        // The exposure and gain shown are the guide camera's, so the flush that
        // precedes a start has to carry its role or it would overwrite the imaging
        // camera's values with them.
        it('flushes the exposure and gain against the guide profile', async () => {
            const wrapper = mountGuideSelected({cameraPhase: {[GUIDE.name]: 'idle'}})

            await wrapper.find('.btn-start').trigger('click')
            await flushPromises()

            expect(updateSettings).toHaveBeenCalledWith(
                expect.objectContaining({camera_role: 'guide'}),
            )
        })

        it('stops the guide camera rather than the capture', async () => {
            const wrapper = mountGuideSelected({
                captureState: 'Capturing',
                cameraPhase: {[GUIDE.name]: 'guiding'},
            })

            await wrapper.find('.btn-stop').trigger('click')
            await flushPromises()

            expect(stopCapture).toHaveBeenCalledWith('guide')
        })
    })

    // Nothing the guide camera produces is stacked, and the capture mode is a global
    // setting — offering Wanderer or Stacking here would silently change the imaging
    // camera's mode.
    describe('Capture mode for the guide camera', () => {
        const GUIDE = {id: 'guide-cam', name: 'Guide Cam', role: 'guide'}

        function modeButtons(wrapper) {
            return wrapper.findComponent({name: 'ButtonGroup'})
        }

        it('offers all three modes for the imaging camera', () => {
            const wrapper = mountCaptureControls()

            const labels = modeButtons(wrapper).props('options').map((o) => o.label)
            expect(labels).toEqual(['Live view', 'Wanderer', 'Stacking'])
        })

        it('offers only Live view for the guide camera', () => {
            const wrapper = mountCaptureControls({
                selectedCameraRole: 'guide',
                selectedCamera: GUIDE.id,
                guideCamera: GUIDE,
            })

            const group = modeButtons(wrapper)
            expect(group.props('options').map((o) => o.label)).toEqual(['Live view'])
            expect(group.props('modelValue')).toBe('off')
            expect(group.props('disabled')).toBe(true)
        })

        // Even in Stacking, which is what the imaging camera is doing behind it.
        it('shows Live view for the guide camera while the rig is stacking', () => {
            const wrapper = mountCaptureControls({
                settings: {stacking: true, wanderer_mode: false},
                selectedCameraRole: 'guide',
                selectedCamera: GUIDE.id,
                guideCamera: GUIDE,
            })

            expect(modeButtons(wrapper).props('modelValue')).toBe('off')
        })

        it('disables the stacking type selector for the guide camera', () => {
            const wrapper = mountCaptureControls({
                selectedCameraRole: 'guide',
                selectedCamera: GUIDE.id,
                guideCamera: GUIDE,
            })

            expect(wrapper.find('.type-select').element.disabled).toBe(true)
        })
    })

    describe('Exposure Control', () => {
        it('syncs exposure in seconds for values >= 1s', async () => {
            const wrapper = mountCaptureControls({
                settings: {exposure_us: 2000000},
            })
            await flushPromises()

            expect(wrapper.find('input[type="number"]').element.value).toBe('2')
            expect(wrapper.find('.input-group select').element.value).toBe('s')
        })

        it('syncs exposure in ms for values >= 1ms', async () => {
            const wrapper = mountCaptureControls({
                settings: {exposure_us: 500000},
            })
            await flushPromises()

            expect(wrapper.find('.input-group input[type="number"]').element.value).toBe('500')
            expect(wrapper.find('.input-group select').element.value).toBe('ms')
        })

        it('calls updateSettings when exposure is changed', async () => {
            const wrapper = mountCaptureControls()

            const exposureInput = wrapper.find('input[type="number"]')
            await exposureInput.setValue(2)
            await exposureInput.trigger('change')
            await flushPromises()

            expect(updateSettings).toHaveBeenCalledWith(
                expect.objectContaining({exposure_us: expect.any(Number)})
            )
        })

        it('applies preset exposure when preset button clicked', async () => {
            const wrapper = mountCaptureControls()

            const presets = wrapper.findAll('.btn-preset')
            // With 1s (1000000μs) default exposure, unit is 's', so presets show numbers without unit
            // The presets for 's' unit are: 0.5, 1, 2, 3, 5, 10, 15, 30, 60
            const preset2 = presets.find((p) => p.text() === '2')

            await preset2.trigger('click')
            await flushPromises()

            expect(updateSettings).toHaveBeenCalledWith({camera_role: 'main', exposure_us: 2000000})
        })
    })

    describe('Gain Control', () => {
        it('syncs current gain value to input', async () => {
            const wrapper = mountCaptureControls({
                settings: {gain: 150},
            })
            await flushPromises()

            const gainInput = wrapper.find('input[type="number"].input-sm')
            expect(gainInput.element.value).toBe('150')
        })

        it('calls updateSettings when gain slider is changed', async () => {
            const wrapper = mountCaptureControls()

            const gainSlider = wrapper.find('input[type="range"]')
            await gainSlider.setValue(200)
            await gainSlider.trigger('change')
            await flushPromises()

            expect(updateSettings).toHaveBeenCalledWith({camera_role: 'main', gain: 200})
        })
    })

    describe('Color mode Control', () => {
        it('syncs auto_stretch value', () => {
            const wrapper = mountCaptureControls({
                settings: {auto_stretch: true},
            })
            const toggle = wrapper.find('.stretch-controls .toggle')
            expect(toggle.element.checked).toBe(true)
        })

        it('calls updateSettings when auto_stretch is toggled', async () => {
            const wrapper = mountCaptureControls({
                settings: {auto_stretch: true},
            })
            const toggle = wrapper.find('.stretch-controls .toggle')
            await toggle.setValue(false)
            await toggle.trigger('change')
            await flushPromises()
            expect(updateSettings).toHaveBeenCalledWith({auto_stretch: false})
        })

        it('shows aggressiveness select when auto_stretch is enabled', async () => {
            const wrapper = mountCaptureControls({
                settings: {auto_stretch: true},
            })
            const select = wrapper.find('.aggressiveness-select')
            expect(select.exists()).toBe(true)
        })

        it('calls updateSettings when aggressiveness is changed', async () => {
            const wrapper = mountCaptureControls({
                settings: {auto_stretch: true, stretch_aggressiveness: 'medium'},
            })
            const select = wrapper.find('.aggressiveness-select')
            await select.setValue('high')
            await select.trigger('change')
            await flushPromises()
            expect(updateSettings).toHaveBeenCalledWith({stretch_aggressiveness: 'high'})
        })
    })

    describe('Error Handling', () => {
        it('clears error when dismiss button clicked', async () => {
            startCapture.mockRejectedValue(new Error('Test error'))

            const wrapper = mountCaptureControls()

            await wrapper.find('.btn-start').trigger('click')
            await flushPromises()

            expect(wrapper.find('.alert-error').exists()).toBe(true)

            await wrapper.find('.btn-close').trigger('click')

            expect(wrapper.find('.alert-error').exists()).toBe(false)
        })
    })
})
