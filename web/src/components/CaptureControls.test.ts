import {mount, flushPromises} from '@vue/test-utils'
import type {VueWrapper} from '@vue/test-utils'
import {readonly, ref} from 'vue'
import CaptureControls from './CaptureControls.vue'
import {BaseProLock} from './ui'
import {HELP_TEXTS} from '../constants'
import {camera, capabilities, settings, stackingTypes} from '../test/fixtures'
import {createEventStreamStub} from '../test/eventStream'
import type {Camera, CameraPhase, CameraRole, Capabilities, CaptureState, Settings} from '../composables/api.types'
import type {ByCameraName} from '../composables/cameraStore'
import type {Option} from '../constants'

// Mock the API module
vi.mock('../composables/api', async (importOriginal) =>
    (await import('../test/mockApi')).mockApi(await importOriginal(), {
        startCapture: vi.fn(),
        stopCapture: vi.fn(),
        updateSettings: vi.fn(),
        getStackingTypes: vi.fn(),
    }))

import {startCapture, stopCapture, updateSettings, getStackingTypes} from '../composables/api'
import {contextProvides} from '../composables/appContext'

describe('CaptureControls', () => {
    beforeEach(() => {
        vi.clearAllMocks()
        // Default mock implementations
        vi.mocked(startCapture).mockResolvedValue({message: 'Started'})
        vi.mocked(stopCapture).mockResolvedValue({message: 'Stopped'})
        vi.mocked(updateSettings).mockResolvedValue(settings())
        vi.mocked(getStackingTypes).mockResolvedValue(stackingTypes())
    })

    interface Overrides {
        /** Over the panel's settings. */
        settings?: Partial<Settings>
        selectedCamera?: string | null
        selectedCameraRole?: CameraRole
        mainCamera?: Camera | null
        guideCamera?: Camera | null
        cameraPhase?: ByCameraName<CameraPhase>
        captureState?: CaptureState
        capabilities?: Capabilities
    }

    function createMockProvides(overrides: Overrides = {}) {
        return {
            settings: ref(settings({
                exposure_us: 1000000,
                gain: 100,
                offset: 10,
                bin: 1,
                auto_stretch: true,
                stacking: true,
                rejection_sigma: 2.5,
                background_subtraction: true,
                ...overrides.settings,
            })),
            selectedCamera: ref('selectedCamera' in overrides ? overrides.selectedCamera ?? null : 'cam1'),
            // Which camera the panel addresses. Start/Stop follows it, so a 'guide'
            // selection drives the guide loop rather than a capture session.
            selectedCameraRole: ref(overrides.selectedCameraRole ?? 'main'),
            mainCamera: ref(
                'mainCamera' in overrides ? overrides.mainCamera ?? null : camera({id: 'cam1', role: 'main'}),
            ),
            guideCamera: ref(overrides.guideCamera ?? null),
            // Keyed by camera *name*: 'guiding' is the phase the backend stamps while
            // the guide loop owns the handle.
            cameraPhase: ref(overrides.cameraPhase ?? {}),
            eventStream: createEventStreamStub({captureState: overrides.captureState ?? 'Idle'}),
            refreshSettings: vi.fn(async () => null),
            capabilities: ref(overrides.capabilities ?? capabilities()),
        }
    }

    function mountCaptureControls(provides: Overrides = {}) {
        return mount(CaptureControls, {
            global: {
                provide: contextProvides(createMockProvides(provides)),
                stubs: {
                    BaseProLock: true,
                },
            },
        })
    }

    describe('Start/Stop Button', () => {
        it('shows Start button when not capturing', () => {
            const wrapper = mountCaptureControls()

            const button = wrapper.find<HTMLButtonElement>('.btn-capture')
            expect(button.classes()).toContain('btn-start')
            expect(button.text()).toContain('Start')
        })

        it('shows Stop button when capturing', () => {
            const wrapper = mountCaptureControls({
                captureState: 'Capturing',
            })

            const button = wrapper.find<HTMLButtonElement>('.btn-capture')
            expect(button.classes()).toContain('btn-stop')
            expect(button.text()).toContain('Stop')
        })

        it('disables Start button when no camera selected', () => {
            const wrapper = mountCaptureControls({
                selectedCamera: null,
            })

            const button = wrapper.find<HTMLButtonElement>('.btn-capture')
            // Vue renders disabled="" for truthy disabled attribute
            expect(button.element.disabled).toBe(true)
        })

        it('disables Stop button when stopping', () => {
            // When stopping, the button shows "Stopping..." but it's still the Start button
            // because isCapturing is false (Stopping != Capturing/Starting)
            const wrapper = mountCaptureControls({
                captureState: 'Stopping',
            })

            const button = wrapper.find<HTMLButtonElement>('.btn-capture')
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
                mainCamera: camera({id: 'imaging-cam', role: 'main'}),
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
            vi.mocked(startCapture).mockRejectedValue(new Error('Camera not ready'))

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
        const GUIDE = camera({id: 'guide-cam', name: 'Guide Cam', role: 'guide'})

        function mountGuideSelected(overrides: Overrides = {}) {
            return mountCaptureControls({
                selectedCameraRole: 'guide',
                selectedCamera: GUIDE.id,
                guideCamera: GUIDE,
                mainCamera: camera({id: 'imaging-cam', role: 'main'}),
                ...overrides,
            })
        }

        it('offers Start while the guide loop is not running', () => {
            const wrapper = mountGuideSelected({cameraPhase: {[GUIDE.name]: 'idle'}})

            const button = wrapper.find<HTMLButtonElement>('.btn-capture')
            expect(button.classes()).toContain('btn-start')
            expect(button.text()).toContain('Start guide')
            expect(button.element.disabled).toBe(false)
        })

        it('offers Stop while the guide loop is running', () => {
            const wrapper = mountGuideSelected({cameraPhase: {[GUIDE.name]: 'guiding'}})

            const button = wrapper.find<HTMLButtonElement>('.btn-capture')
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
        const GUIDE = camera({id: 'guide-cam', name: 'Guide Cam', role: 'guide'})

        function modeButtons(wrapper: VueWrapper) {
            return wrapper.findComponent({name: 'ButtonGroup'})
        }

        function labels(group: ReturnType<typeof modeButtons>) {
            const options: readonly Option<string>[] = group.props('options')
            return options.map((o) => o.label)
        }

        it('offers all three modes for the imaging camera', () => {
            const wrapper = mountCaptureControls()

            expect(labels(modeButtons(wrapper))).toEqual(['Live view', 'Wanderer', 'Stacking'])
        })

        it('offers only Live view for the guide camera', () => {
            const wrapper = mountCaptureControls({
                selectedCameraRole: 'guide',
                selectedCamera: GUIDE.id,
                guideCamera: GUIDE,
            })

            const group = modeButtons(wrapper)
            expect(labels(group)).toEqual(['Live view'])
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

            expect(wrapper.find<HTMLSelectElement>('.type-select').element.disabled).toBe(true)
        })
    })

    describe('Exposure Control', () => {
        it('syncs exposure in seconds for values >= 1s', async () => {
            const wrapper = mountCaptureControls({
                settings: {exposure_us: 2000000},
            })
            await flushPromises()

            expect(wrapper.find<HTMLInputElement>('input[type="number"]').element.value).toBe('2')
            expect(wrapper.find<HTMLSelectElement>('.input-group select').element.value).toBe('s')
        })

        it('syncs exposure in ms for values >= 1ms', async () => {
            const wrapper = mountCaptureControls({
                settings: {exposure_us: 500000},
            })
            await flushPromises()

            expect(wrapper.find<HTMLInputElement>('.input-group input[type="number"]').element.value).toBe('500')
            expect(wrapper.find<HTMLSelectElement>('.input-group select').element.value).toBe('ms')
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

            await preset2?.trigger('click')
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

            const gainInput = wrapper.find<HTMLInputElement>('input[type="number"].input-sm')
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

        // The number input is the value display; the label's read-only copy would be a
        // second one a few pixels away, which reads as two different settings.
        it('shows the gain value once, in the number input', async () => {
            const wrapper = mountCaptureControls({settings: {gain: 46}})
            await flushPromises()

            const gainGroup = wrapper.find('.slider-control-wrapper')
            expect(gainGroup.find<HTMLInputElement>('input[type="number"].input-sm').element.value).toBe('46')
            expect(gainGroup.find('.current-value').exists()).toBe(false)
        })
    })

    describe('Focus/Finder mode', () => {
        it('syncs focus_mode value', async () => {
            const wrapper = mountCaptureControls({settings: {focus_mode: true}})
            await flushPromises()

            expect(wrapper.find<HTMLInputElement>('.focus-mode-controls .toggle').element.checked).toBe(true)
        })

        it('is off at the server\'s default', async () => {
            const wrapper = mountCaptureControls()
            await flushPromises()

            expect(wrapper.find<HTMLInputElement>('.focus-mode-controls .toggle').element.checked).toBe(false)
        })

        // A pipeline setting, not a hardware one: a `camera_role` would send it down the
        // per-camera profile path, where it does not belong.
        it('calls updateSettings without a camera role when toggled', async () => {
            const wrapper = mountCaptureControls({settings: {focus_mode: false}})
            await flushPromises()

            const toggle = wrapper.find('.focus-mode-controls .toggle')
            await toggle.setValue(true)
            await flushPromises()

            expect(updateSettings).toHaveBeenCalledWith({focus_mode: true})
        })

        it('turns the mode back off', async () => {
            const wrapper = mountCaptureControls({settings: {focus_mode: true}})
            await flushPromises()

            const toggle = wrapper.find('.focus-mode-controls .toggle')
            await toggle.setValue(false)
            await flushPromises()

            expect(updateSettings).toHaveBeenCalledWith({focus_mode: false})
        })

        // The six settings are greyed in the Settings panel off the *server* value, so
        // an optimistic local ref left behind by a failed save has the two panels
        // disagreeing about the same setting.
        it('puts the toggle back when the save fails', async () => {
            vi.mocked(updateSettings).mockRejectedValue(new Error('Network down'))
            const provides = createMockProvides({settings: {focus_mode: false}})
            const wrapper = mount(CaptureControls, {global: {provide: contextProvides(provides)}})
            await flushPromises()

            await wrapper.find('.focus-mode-controls .toggle').setValue(true)
            await flushPromises()

            expect(wrapper.find('.alert-error').exists()).toBe(true)
            expect(wrapper.find<HTMLInputElement>('.focus-mode-controls .toggle').element.checked).toBe(
                provides.settings.value.focus_mode,
            )
        })

        // One of the six runs before demosaic, so the frames the accumulator integrates
        // lose their banding correction — and averaging is exactly what cannot take that
        // back out. The backend answers 409; the toggle never gets there.
        it('is disabled while a stacking capture runs', async () => {
            const wrapper = mountCaptureControls({
                captureState: 'Capturing',
                settings: {focus_mode: false, stacking: true, wanderer_mode: false},
            })
            await flushPromises()

            expect(
                wrapper.find('.focus-mode-controls .toggle').attributes('disabled'),
            ).toBeDefined()
        })

        it('is disabled while a wanderer capture runs', async () => {
            const wrapper = mountCaptureControls({
                captureState: 'Capturing',
                settings: {focus_mode: false, stacking: true, wanderer_mode: true},
            })
            await flushPromises()

            expect(
                wrapper.find('.focus-mode-controls .toggle').attributes('disabled'),
            ).toBeDefined()
        })

        // Planetary never runs row/column pattern removal, so the mode cannot touch its stack.
        it('stays available during a planetary stacking capture', async () => {
            const wrapper = mountCaptureControls({
                captureState: 'Capturing',
                settings: {
                    focus_mode: false,
                    stacking: true,
                    wanderer_mode: false,
                    stacking_type: 'planetary',
                },
            })
            await flushPromises()

            expect(
                wrapper.find('.focus-mode-controls .toggle').attributes('disabled'),
            ).toBeUndefined()
        })

        // The guard is about the accumulator, not about the camera being busy.
        it('stays available during a live view capture', async () => {
            const wrapper = mountCaptureControls({
                captureState: 'Capturing',
                settings: {focus_mode: false, stacking: false},
            })
            await flushPromises()

            expect(
                wrapper.find('.focus-mode-controls .toggle').attributes('disabled'),
            ).toBeUndefined()
        })

        // Never trap the observer in the mode.
        it('can always be switched off, even mid-stack', async () => {
            const wrapper = mountCaptureControls({
                captureState: 'Capturing',
                settings: {focus_mode: true, stacking: true},
            })
            await flushPromises()

            const toggle = wrapper.find('.focus-mode-controls .toggle')
            expect(toggle.attributes('disabled')).toBeUndefined()

            await toggle.setValue(false)
            await flushPromises()

            expect(updateSettings).toHaveBeenCalledWith({focus_mode: false})
        })

        // Framing is exactly when the mode is wanted, and the guide camera is what you
        // frame with — so unlike exposure and gain it is not scoped to the selection.
        it('stays available while the guide camera is selected', async () => {
            const wrapper = mountCaptureControls({
                selectedCameraRole: 'guide',
                guideCamera: camera({id: 'cam2', name: 'guide', role: 'guide'}),
                settings: {focus_mode: false},
            })
            await flushPromises()

            const toggle = wrapper.find('.focus-mode-controls .toggle')
            expect(toggle.attributes('disabled')).toBeUndefined()

            await toggle.setValue(true)
            await flushPromises()

            expect(updateSettings).toHaveBeenCalledWith({focus_mode: true})
        })
    })

    describe('Color mode Control', () => {
        it('syncs auto_stretch value', () => {
            const wrapper = mountCaptureControls({
                settings: {auto_stretch: true},
            })
            const toggle = wrapper.find<HTMLInputElement>('.stretch-controls .toggle')
            expect(toggle.element.checked).toBe(true)
        })

        it('calls updateSettings when auto_stretch is toggled', async () => {
            const wrapper = mountCaptureControls({
                settings: {auto_stretch: true},
            })
            const toggle = wrapper.find<HTMLInputElement>('.stretch-controls .toggle')
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

        it('hides the aggressiveness select for Planetary, which sets its own levels', async () => {
            const wrapper = mountCaptureControls({
                settings: {auto_stretch: true, stacking_type: 'planetary'},
            })
            await flushPromises()
            expect(wrapper.find('.aggressiveness-select').exists()).toBe(false)
            expect(wrapper.find('.stretch-controls .toggle').exists()).toBe(true)
        })

        it('shows the select again when the type changes from Planetary', async () => {
            const wrapper = mountCaptureControls({
                settings: {auto_stretch: true, stacking_type: 'planetary'},
            })
            await flushPromises()
            await wrapper.find('.type-select').setValue('deep_sky')
            await flushPromises()
            expect(wrapper.find('.aggressiveness-select').exists()).toBe(true)
            expect(updateSettings).not.toHaveBeenCalledWith(expect.objectContaining({stretch_aggressiveness: expect.anything()}))
        })

        it.each(['deep_sky', 'comet'] as const)('keeps the select for %s', async (stackingType) => {
            const wrapper = mountCaptureControls({
                settings: {auto_stretch: true, stacking_type: stackingType},
            })
            await flushPromises()
            expect(wrapper.find('.aggressiveness-select').exists()).toBe(true)
        })

        it('explains Planetary\'s own levels in the Color mode help', async () => {
            const wrapper = mountCaptureControls({
                settings: {auto_stretch: true, stacking_type: 'planetary'},
            })
            await flushPromises()
            const icon = wrapper.findAllComponents({name: 'BaseInfoIcon'})
                .find((c) => c.props('message') === HELP_TEXTS.auto_stretch_planetary)
            expect(icon).toBeDefined()
        })

        it('calls updateSettings when aggressiveness is changed', async () => {
            const wrapper = mountCaptureControls({
                settings: {auto_stretch: true, stretch_aggressiveness: 'medium'},
            })
            const select = wrapper.find('.aggressiveness-select')
            await select.setValue('low')
            await select.trigger('change')
            await flushPromises()
            expect(updateSettings).toHaveBeenCalledWith({stretch_aggressiveness: 'low'})
        })
    })

    describe('Error Handling', () => {
        it('clears error when dismiss button clicked', async () => {
            vi.mocked(startCapture).mockRejectedValue(new Error('Test error'))

            const wrapper = mountCaptureControls()

            await wrapper.find('.btn-start').trigger('click')
            await flushPromises()

            expect(wrapper.find('.alert-error').exists()).toBe(true)

            await wrapper.find('.btn-close').trigger('click')

            expect(wrapper.find('.alert-error').exists()).toBe(false)
        })
    })

    // App provides the capabilities as a read-only ref, which a script-side read must unwrap:
    // reading it as a plain object locked comet stacking for every Pro user.
    describe('Comet Pro lock', () => {
        const comet = (proStacking: boolean) => ({
            settings: {stacking_type: 'comet'},
            capabilities: capabilities({comet: {pro_stacking: proStacking}}),
        }) satisfies Overrides

        it('locks comet stacking without the Pro comet plugin', async () => {
            const wrapper = mountCaptureControls(comet(false))
            await flushPromises()
            expect(wrapper.find('.comet-pro-lock').exists()).toBe(true)
        })

        // The lock once took `title`/`message` it does not declare: the help never showed,
        // and the title replaced the tooltip naming the feature.
        it('says what Pro adds and names the feature on the lock', async () => {
            const wrapper = mountCaptureControls(comet(false))
            await flushPromises()
            const lock = wrapper.find('.comet-pro-lock')
            expect(lock.text()).toContain(HELP_TEXTS.comet_pro)
            expect(wrapper.findComponent(BaseProLock).props('feature')).toBe('Comet stacking')
        })

        it('unlocks it for Pro when the capabilities arrive as App provides them', async () => {
            const wrapper = mount(CaptureControls, {
                global: {
                    provide: contextProvides({
                        ...createMockProvides(comet(true)),
                        capabilities: readonly(ref(comet(true).capabilities)),
                    }),
                    stubs: {BaseProLock: true},
                },
            })
            await flushPromises()
            expect(wrapper.find('.comet-pro-lock').exists()).toBe(false)
        })
    })

    it("offers the server's stacking types by name", async () => {
        const wrapper = mountCaptureControls()
        await flushPromises()

        const options = wrapper.findAll<HTMLOptionElement>('.type-select option')
        expect(options.map((option) => option.text())).toEqual(['Deep Sky', 'Planetary', 'Comet'])
        expect(options.map((option) => option.element.value)).toEqual(['deep_sky', 'planetary', 'comet'])
    })

    // An empty Type select said nothing about why; the failure was only logged.
    it('says so when the stacking types cannot be loaded', async () => {
        vi.mocked(getStackingTypes).mockRejectedValueOnce(new Error('Server unavailable'))
        const wrapper = mountCaptureControls()
        await flushPromises()

        expect(wrapper.text()).toContain('Could not load the stacking types: Server unavailable')
    })

    it('starts with the exposure as typed, in whole microseconds', async () => {
        const wrapper = mountCaptureControls()
        await flushPromises()
        const field = wrapper.find<HTMLInputElement>('.input-group input[type="number"]')

        field.element.value = '4.1'
        await field.trigger('input')
        await wrapper.find('.btn-start').trigger('click')
        await flushPromises()

        expect(updateSettings).toHaveBeenCalledWith(expect.objectContaining({exposure_us: 4100000}))
        expect(startCapture).toHaveBeenCalled()
    })
})
