import {describe, it, expect, vi, beforeEach} from 'vitest'
import {mount, flushPromises} from '@vue/test-utils'
import {ref} from 'vue'
import SettingsPanel from './SettingsPanel.vue'
import {DEFAULT_SETTINGS} from '../constants/index.js'

// Mock the API module
vi.mock('../composables/api.js', () => ({
    updateSettings: vi.fn(),
}))

import {updateSettings} from '../composables/api.js'

/**
 * A toggle's checkbox, found by the label text next to it.
 *
 * `findAll('.toggle')[n]` was how these were selected; every control added above
 * one silently retargeted the assertion at a different setting.
 */
function findToggleByLabel(wrapper, label) {
    const found = wrapper
        .findAll('.toggle-label')
        .find((el) => el.text().includes(label))
    if (!found) throw new Error(`no toggle labelled "${label}"`)
    return found.find('input[type="checkbox"]')
}

describe('SettingsPanel', () => {
    beforeEach(() => {
        vi.clearAllMocks()
        updateSettings.mockResolvedValue({})
    })

    function createDefaultSettings() {
        return {
            offset: 10,
            bin: 1,
            auto_stretch: true,
            auto_stretch_intensity: 0.3,
            stacking: true,
            background_subtraction: true,
            raw_frame_saving: {live_view: false, wanderer: false, stacking: false},
            save_stacked_image: false,
        }
    }

    function createMockProvides(overrides = {}) {
        return {
            // `settings: null` mounts the panel as it exists before the first payload
            // arrives, which is a state the panel really renders in.
            settings: ref(
                overrides.settings === null
                    ? null
                    : {...createDefaultSettings(), ...overrides.settings}
            ),
            refreshSettings: vi.fn().mockResolvedValue(undefined),
            simulatorEnabled: ref(overrides.simulatorEnabled ?? false),
            capabilities: ref({
                has_pro: false,
                deep_sky: {advanced_rejection: false, rbf_background: false},
                planetary: {advanced_stacking: false},
                push_to: {astap_solver: false},
                ...overrides.capabilities,
            }),
        }
    }

    function mountSettingsPanel(provides = {}) {
        return mount(SettingsPanel, {
            global: {
                provide: createMockProvides(provides),
                stubs: {
                    BaseProLock: true,
                },
            },
        })
    }

    describe('Slider Constants', () => {
        it('uses correct constants for Black level and Auto Stretch sliders', () => {
            const wrapper = mountSettingsPanel()
            const sliders = wrapper.findAllComponents({ name: 'BaseSlider' })
            
            const autoStretchSlider = sliders.find(s => s.props('label') === 'Color Intensity')
            expect(autoStretchSlider.exists()).toBe(true)
            expect(autoStretchSlider.props('min')).toBe(0.0)
            expect(autoStretchSlider.props('max')).toBe(1.0)
            
            const blackLevelSlider = sliders.find(s => s.props('label') === 'Black level')
            expect(blackLevelSlider.exists()).toBe(true)
            expect(blackLevelSlider.props('min')).toBe(0.0)
            expect(blackLevelSlider.props('max')).toBe(1.0)
        })
    })

    describe('Advanced Settings - Binning', () => {
        it('displays binning options', () => {
            const wrapper = mountSettingsPanel()

            // Find the binning buttons in the panel
            const binButtons = wrapper.findAll('.btn-option').filter(b =>
                ['1x1', '2x2', '3x3', '4x4'].includes(b.text())
            )
            expect(binButtons.length).toBe(4)
            expect(binButtons.map((b) => b.text())).toEqual(['1x1', '2x2', '3x3', '4x4'])
        })

        it('highlights active binning option', () => {
            const wrapper = mountSettingsPanel({
                settings: {bin: 2},
            })

            const binButtons = wrapper.findAll('.btn-option').filter(b =>
                ['1x1', '2x2', '3x3', '4x4'].includes(b.text())
            )
            expect(binButtons[1].classes()).toContain('active')
        })

        it('updates binning when button clicked', async () => {
            const wrapper = mountSettingsPanel()

            const binButtons = wrapper.findAll('.btn-option').filter(b =>
                ['1x1', '2x2', '3x3', '4x4'].includes(b.text())
            )
            await binButtons[2].trigger('click') // 3x3
            await flushPromises()

            expect(updateSettings).toHaveBeenCalledWith({bin: 3})
        })
    })

    describe('Processing Settings Section', () => {
        it('updates background_subtraction when toggled', async () => {
            const wrapper = mountSettingsPanel({
                settings: {background_subtraction: true},
            })

            // Selected by label, not by index: a positional lookup breaks every
            // time a control is added above this one, which it has twice now.
            const bgSubToggle = findToggleByLabel(wrapper, 'Background Subtraction')

            await bgSubToggle.setValue(false)
            await bgSubToggle.trigger('change')
            await flushPromises()

            expect(updateSettings).toHaveBeenCalledWith({background_subtraction: false})
        })

        it('has an Color Intensity slider', () => {
            const wrapper = mountSettingsPanel()
            // Using deep search to find the label text
            expect(wrapper.html()).toContain('Color Intensity')
        })
    })

    describe('Stacking Settings Section', () => {
        // Note: stacking toggle moved to CaptureControls.vue

        it('shows stacking options when stacking is enabled', () => {
            const wrapper = mountSettingsPanel({
                settings: {stacking: true},
            })

            const sigmaLabel = wrapper
                .findAll('.control-label')
                .find((l) => l.text().includes('Frame Weighting'))

            expect(sigmaLabel).toBeTruthy()
        })

        it('hides stacking options when stacking is disabled', () => {
            const wrapper = mountSettingsPanel({
                settings: {stacking: false},
            })

            const sigmaLabel = wrapper
                .findAll('.control-label')
                .find((l) => l.text().includes('Frame Weighting'))
            expect(sigmaLabel).toBeFalsy()
        })

        it('updates weighting_preset when select changed', async () => {
            const wrapper = mountSettingsPanel({
                settings: {stacking: true, weighting_preset: 'balanced'},
            })

            const select = wrapper.find('#weighting-preset-select')
            await select.setValue('galaxies')
            await select.trigger('change')
            await flushPromises()

            expect(updateSettings).toHaveBeenCalledWith({weighting_preset: 'galaxies'})
        })
    })

    describe('Error Handling', () => {
        it('shows error when updateSettings fails', async () => {
            updateSettings.mockRejectedValue(new Error('Update failed'))

            const wrapper = mountSettingsPanel()

            const binButtons = wrapper.findAll('.btn-option')
            await binButtons[1].trigger('click')
            await flushPromises()

            expect(wrapper.find('.alert-error').text()).toContain('Update failed')
        })

        it('clears error when dismiss button clicked', async () => {
            updateSettings.mockRejectedValue(new Error('Test error'))

            const wrapper = mountSettingsPanel()

            const binButtons = wrapper.findAll('.btn-option')
            await binButtons[1].trigger('click')
            await flushPromises()

            expect(wrapper.find('.alert-error').exists()).toBe(true)

            await wrapper.find('.btn-close').trigger('click')

            expect(wrapper.find('.alert-error').exists()).toBe(false)
        })
    })

    describe('Advanced Settings Section', () => {
        it('displays simulated camera toggle', () => {
            const wrapper = mountSettingsPanel()

            const advancedSection = wrapper.findAll('.section-title').find((s) => s.text() === 'Advanced')
            expect(advancedSection).toBeTruthy()

            const toggleTexts = wrapper.findAll('.toggle-text')
            const simulatorToggle = toggleTexts.find((t) => t.text() === 'Simulated Camera')
            expect(simulatorToggle).toBeTruthy()
        })

        it('updates simulatorEnabled when toggled', async () => {
            const simulatorEnabled = ref(false)
            const wrapper = mount(SettingsPanel, {
                global: {
                    provide: {
                        settings: ref(createDefaultSettings()),
                        refreshSettings: vi.fn(),
                        simulatorEnabled,
                        capabilities: ref({
                            has_pro: false,
                            deep_sky: {advanced_rejection: false, rbf_background: false},
                            planetary: {advanced_stacking: false},
                            push_to: {astap_solver: false},
                        }),
                    },
                },
            })

            // Find simulator toggle by data-test attribute, then find its inner input
            const simulatorToggle = wrapper.find('[data-test="simulator-toggle"]').find('input')

            await simulatorToggle.setValue(true)
            await flushPromises()

            expect(simulatorEnabled.value).toBe(true)
        })
    })

    describe('Settings Sync', () => {
        it('updates local state when settings prop changes', async () => {
            const settings = ref(createDefaultSettings())
            const wrapper = mount(SettingsPanel, {
                global: {
                    provide: {
                        settings,
                        refreshSettings: vi.fn(),
                        simulatorEnabled: ref(false),
                        capabilities: ref({
                            has_pro: false,
                            deep_sky: {advanced_rejection: false, rbf_background: false},
                            planetary: {advanced_stacking: false},
                            push_to: {astap_solver: false},
                        }),
                    },
                },
            })

            // Initial state
            let binButtons = wrapper.findAll('.btn-option').filter(b =>
                ['1x1', '2x2', '3x3', '4x4'].includes(b.text())
            )
            expect(binButtons[0].classes()).toContain('active')

            // Update settings externally
            settings.value = {...settings.value, bin: 2}
            await flushPromises()

            // Should reflect new value
            binButtons = wrapper.findAll('.btn-option').filter(b =>
                ['1x1', '2x2', '3x3', '4x4'].includes(b.text())
            )
            expect(binButtons[1].classes()).toContain('active')
        })
    })

    describe('Preview Section', () => {
        it('sends the chosen processing resolution as a bare value', async () => {
            const wrapper = mountSettingsPanel()
            await flushPromises()

            const select = wrapper.find('#preview-resolution-select')
            await select.setValue('qhd1440')
            await flushPromises()

            expect(updateSettings).toHaveBeenCalledWith({preview_resolution: 'qhd1440'})
        })

        it('shows what the server reports rather than the default', async () => {
            const wrapper = mountSettingsPanel({settings: {preview_resolution: 'hd1080'}})
            await flushPromises()

            expect(wrapper.find('#preview-resolution-select').element.value).toBe('hd1080')
        })
    })

    describe('Noise Reduction Section', () => {
        it('sends the whole denoise object when a filter is toggled', async () => {
            const wrapper = mountSettingsPanel()

            await findToggleByLabel(wrapper, 'Background Grain').setValue(false)
            await flushPromises()

            expect(updateSettings).toHaveBeenCalledWith({
                denoise: expect.objectContaining({luma: false, chroma: true}),
            })
        })

        it('hides a filter\'s sliders while that filter is off', async () => {
            const wrapper = mountSettingsPanel({
                settings: {
                    denoise: {
                        chroma: true,
                        chroma_strength: 1.0,
                        luma: false,
                        luma_strength: 1.0,
                        star_protection: 1.0,
                    },
                },
            })
            await flushPromises()

            const labels = wrapper
                .findAllComponents({name: 'BaseSlider'})
                .map((s) => s.props('label'))
            expect(labels).toContain('Colour strength')
            expect(labels).not.toContain('Structure strength')
            expect(labels).not.toContain('Star protection')
        })

        // The only control that moves sky grain much, and it was previously
        // hardcoded at full protection with no way to reach it.
        it('offers star protection, and sends it with the rest of the object', async () => {
            const wrapper = mountSettingsPanel()
            await flushPromises()

            const slider = wrapper
                .findAllComponents({name: 'BaseSlider'})
                .find((s) => s.props('label') === 'Star protection')
            expect(slider).toBeDefined()
            expect(slider.props('max')).toBe(1.0)

            slider.vm.$emit('update:modelValue', 0.4)
            slider.vm.$emit('change')
            await flushPromises()

            expect(updateSettings).toHaveBeenCalledWith({
                denoise: expect.objectContaining({star_protection: 0.4}),
            })
        })

        // The manual tells the observer to raise this one; a slider that stopped
        // at its own default could not be raised at all.
        it('lets structure strength go above the tuned default', async () => {
            const wrapper = mountSettingsPanel()
            await flushPromises()

            const slider = wrapper
                .findAllComponents({name: 'BaseSlider'})
                .find((s) => s.props('label') === 'Structure strength')
            expect(slider.props('max')).toBe(2.0)
        })

        it('falls back to defaults when the server sends no denoise settings', async () => {
            const wrapper = mountSettingsPanel()
            await flushPromises()

            expect(findToggleByLabel(wrapper, 'Colour Mottle').element.checked).toBe(true)
            expect(findToggleByLabel(wrapper, 'Background Grain').element.checked).toBe(true)
        })
    })

    describe('Storage Section', () => {
        // Raw frames can now be saved in any mode, so the section is no longer gated on
        // being in Stacking - only the stacked-image switch still is.
        const modes = [
            ['live view', {stacking: false, wanderer_mode: false}],
            ['wanderer', {stacking: true, wanderer_mode: true}],
            ['stacking', {stacking: true, wanderer_mode: false}],
        ]

        it.each(modes)('shows the storage section in %s mode', (_name, settings) => {
            const wrapper = mountSettingsPanel({settings})

            const storageTitle = wrapper
                .findAll('.section-title')
                .find((s) => s.text() === 'Storage')
            expect(storageTitle).toBeTruthy()
        })

        it.each(modes)('offers all three raw-frame switches in %s mode', (_name, settings) => {
            const wrapper = mountSettingsPanel({settings})

            expect(findToggleByLabel(wrapper, 'Live view')).toBeTruthy()
            expect(findToggleByLabel(wrapper, 'Wanderer')).toBeTruthy()
            expect(findToggleByLabel(wrapper, 'Stacking')).toBeTruthy()
        })

        it('hides the stacked-image switch outside stacking mode', () => {
            const wrapper = mountSettingsPanel({
                settings: {stacking: true, wanderer_mode: true},
            })

            expect(() => findToggleByLabel(wrapper, 'Save Stacked Image')).toThrow()
        })

        // The server takes the group whole, so a partial object would clear the modes
        // the user did not touch.
        it('sends the whole selection when one mode is toggled', async () => {
            const wrapper = mountSettingsPanel({
                settings: {
                    raw_frame_saving: {live_view: false, wanderer: false, stacking: true},
                },
            })

            const wandererToggle = findToggleByLabel(wrapper, 'Wanderer')
            await wandererToggle.setValue(true)
            await wandererToggle.trigger('change')
            await flushPromises()

            expect(updateSettings).toHaveBeenCalledWith({
                raw_frame_saving: {live_view: false, wanderer: true, stacking: true},
            })
        })

        // A shallow spread of DEFAULT_SETTINGS hands the panel a reference into the
        // shared constant, and the section renders before the first settings payload
        // arrives - so a click in that window used to edit the defaults every later
        // fallback reads.
        it('does not write through to the shared defaults before settings arrive', async () => {
            const wrapper = mountSettingsPanel({settings: null})

            const liveToggle = findToggleByLabel(wrapper, 'Live view')
            await liveToggle.setValue(true)
            await liveToggle.trigger('change')
            await flushPromises()

            expect(DEFAULT_SETTINGS.raw_frame_saving).toEqual({
                live_view: false,
                wanderer: false,
                stacking: false,
            })
        })

        it('reflects the saved selection in the switches', () => {
            const wrapper = mountSettingsPanel({
                settings: {
                    raw_frame_saving: {live_view: true, wanderer: false, stacking: true},
                },
            })

            expect(findToggleByLabel(wrapper, 'Live view').element.checked).toBe(true)
            expect(findToggleByLabel(wrapper, 'Wanderer').element.checked).toBe(false)
            expect(findToggleByLabel(wrapper, 'Stacking').element.checked).toBe(true)
        })
    })

    // One signed slider drives two different transforms, and the toggle belongs
    // to only one of them. Leaving it live on the other half offers a control
    // that does nothing.
    describe('Eyepiece black floor', () => {
        function blackFloorSlider(wrapper) {
            return wrapper
                .findAllComponents({name: 'BaseSlider'})
                .find((s) => s.props('label') === 'Black floor')
        }

        function darkerSkyToggle(wrapper) {
            return wrapper
                .findAllComponents({name: 'BaseToggle'})
                .find((t) => t.props('label') === 'Darker sky')
        }

        it('lets the slider reach the darkening half', () => {
            const wrapper = mountSettingsPanel()
            const slider = blackFloorSlider(wrapper)
            expect(slider).toBeDefined()
            expect(slider.props('min')).toBe(-0.09)
            expect(slider.props('max')).toBe(0.15)
        })

        it('disables Darker sky while the floor is on its lifting half', async () => {
            const wrapper = mountSettingsPanel({
                settings: {eyepiece: {black_floor: 0.04, darker_sky: false}},
            })
            await flushPromises()

            expect(darkerSkyToggle(wrapper).props('disabled')).toBe(true)
        })

        it('enables Darker sky once the floor goes negative', async () => {
            const wrapper = mountSettingsPanel({
                settings: {eyepiece: {black_floor: -0.05, darker_sky: false}},
            })
            await flushPromises()

            expect(darkerSkyToggle(wrapper).props('disabled')).toBe(false)
        })

        it('sends the whole eyepiece object when Darker sky is toggled', async () => {
            const wrapper = mountSettingsPanel({
                settings: {eyepiece: {black_floor: -0.05, darker_sky: false}},
            })
            await flushPromises()

            await findToggleByLabel(wrapper, 'Darker sky').setValue(true)
            await flushPromises()

            expect(updateSettings).toHaveBeenCalledWith({
                eyepiece: expect.objectContaining({
                    black_floor: -0.05,
                    darker_sky: true,
                }),
            })
        })
    })
})
