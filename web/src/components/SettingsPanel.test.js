import {describe, it, expect, vi, beforeEach} from 'vitest'
import {mount, flushPromises} from '@vue/test-utils'
import {ref} from 'vue'
import SettingsPanel from './SettingsPanel.vue'

// Mock the API module
vi.mock('../composables/api.js', () => ({
    updateSettings: vi.fn(),
}))

import {updateSettings} from '../composables/api.js'

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
            stacking: true,
            background_subtraction: true,
            save_raw_frames: false,
            save_stacked_image: false,
        }
    }

    function createMockProvides(overrides = {}) {
        return {
            settings: ref({
                ...createDefaultSettings(),
                ...overrides.settings,
            }),
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

            // Find the background subtraction toggle (first processing toggle now)
            const toggles = wrapper.findAll('.toggle')
            const bgSubToggle = toggles[0] // was [1] when auto_stretch was there

            await bgSubToggle.setValue(false)
            await bgSubToggle.trigger('change')
            await flushPromises()

            expect(updateSettings).toHaveBeenCalledWith({background_subtraction: false})
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

            // Find simulator toggle (last toggle in the panel)
            const toggles = wrapper.findAll('.toggle')
            const simulatorToggle = toggles[toggles.length - 1]

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

    describe('Storage Section Visibility', () => {
        it('hides storage section in live view mode (stacking disabled)', () => {
            const wrapper = mountSettingsPanel({
                settings: {stacking: false, wanderer_mode: false},
            })

            const storageTitle = wrapper
                .findAll('.section-title')
                .find((s) => s.text() === 'Storage')
            expect(storageTitle).toBeFalsy()
        })

        it('hides storage section in wanderer mode', () => {
            const wrapper = mountSettingsPanel({
                settings: {stacking: true, wanderer_mode: true},
            })

            const storageTitle = wrapper
                .findAll('.section-title')
                .find((s) => s.text() === 'Storage')
            expect(storageTitle).toBeFalsy()
        })

        it('shows storage section in stacking mode', () => {
            const wrapper = mountSettingsPanel({
                settings: {stacking: true, wanderer_mode: false},
            })

            const storageTitle = wrapper
                .findAll('.section-title')
                .find((s) => s.text() === 'Storage')
            expect(storageTitle).toBeTruthy()
        })
    })
})
