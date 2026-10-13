/** The Noise Reduction section (`ImageQualitySettings`) and the switches Focus/Finder mode holds, through the panel's save path. */
import {describe, it, expect, vi, beforeEach} from 'vitest'
import {flushPromises} from '@vue/test-utils'
import type {VueWrapper} from '@vue/test-utils'
import BaseSlider from './ui/BaseSlider.vue'
import {settings} from '../test/fixtures.js'
import {findSliderByLabel, findToggleByLabel, mountSettingsPanel} from '../test/settingsPanel.js'
import type * as Api from '../composables/api.js'

vi.mock('../composables/api.js', async (importOriginal) =>
    (await import('../test/mockApi.js')).mockApi(await importOriginal<typeof Api>(), {
        updateSettings: vi.fn(),
        getAiCompute: vi.fn(),
        remeasureAiCompute: vi.fn(),
    }))

import {updateSettings} from '../composables/api.js'

describe('SettingsPanel noise reduction', () => {
    beforeEach(() => {
        vi.clearAllMocks()
        vi.mocked(updateSettings).mockResolvedValue(settings())
    })

    describe('Noise Reduction Section', () => {
        const WITH_DENOISE = {
            capabilities: {deep_sky: {advanced_rejection: false, rbf_background: false, denoise: true}},
        }

        const denoiseControls = (wrapper: VueWrapper) => [
            findToggleByLabel(wrapper, 'Colour Mottle').attributes('disabled'),
            ...wrapper
                .findAllComponents(BaseSlider)
                .filter((s) =>
                    ['Colour strength', 'Background Grain', 'Structure strength', 'Detail'].includes(
                        s.props('label') ?? ''
                    )
                )
                .map((s) => (s.props('disabled') ? '' : undefined)),
        ]

        // Denoising is a Pro feature: the filters live in the Pro plugin, and so does
        // every control over them. The section stays visible, locked, so the build says
        // what it does not do rather than silently missing a section.
        it('locks every noise-reduction control without the denoise capability', () => {
            const wrapper = mountSettingsPanel()

            expect(findToggleByLabel(wrapper, 'Denoise').attributes('disabled')).toBeDefined()
            expect(denoiseControls(wrapper).every((d) => d !== undefined)).toBe(true)
        })

        it('turning Denoise off holds the filter controls but not the switch itself', async () => {
            const wrapper = mountSettingsPanel({
                ...WITH_DENOISE,
                settings: {
                    denoise: {
                        enabled: false,
                        chroma: true,
                        chroma_strength: 1.0,
                        background_grain: 0.5,
                        luma_strength: 1.0,
                    },
                },
            })
            await flushPromises()

            expect(findToggleByLabel(wrapper, 'Denoise').attributes('disabled')).toBeUndefined()
            expect(denoiseControls(wrapper).every((d) => d !== undefined)).toBe(true)
        })

        it('sends the switch with the rest of the denoise object', async () => {
            const wrapper = mountSettingsPanel(WITH_DENOISE)

            await findToggleByLabel(wrapper, 'Denoise').setValue(false)
            await flushPromises()

            expect(updateSettings).toHaveBeenCalledWith({
                denoise: expect.objectContaining({enabled: false, background_grain: 0.5}),
            })
        })

        // The mode reaches "off" through the strengths it snapshots. A denoise switch the
        // mode forced false is how a saved file once lost the Background Grain dial for
        // good, so the observer's master switch is deliberately theirs.
        it('Focus/Finder mode never holds the Denoise switch', () => {
            const wrapper = mountSettingsPanel({...WITH_DENOISE, settings: {focus_mode: true}})

            expect(findToggleByLabel(wrapper, 'Denoise').attributes('disabled')).toBeUndefined()
        })

        it('sends the whole denoise object when a filter is toggled', async () => {
            const wrapper = mountSettingsPanel(WITH_DENOISE)

            await findToggleByLabel(wrapper, 'Colour Mottle').setValue(false)
            await flushPromises()

            expect(updateSettings).toHaveBeenCalledWith({
                denoise: expect.objectContaining({chroma: false, background_grain: 0.5}),
            })
        })

        it('hides the colour slider while colour mottle is off', async () => {
            const wrapper = mountSettingsPanel({
                settings: {
                    denoise: {
                        chroma: false,
                        chroma_strength: 1.0,
                        background_grain: 0.5,
                        luma_strength: 1.0,
                    },
                },
            })
            await flushPromises()

            const labels = wrapper
                .findAllComponents(BaseSlider)
                .map((s) => s.props('label'))
            expect(labels).not.toContain('Colour strength')
            expect(labels).toContain('Background Grain')
            expect(labels).toContain('Structure strength')
        })

        // One dial over the wavelet's finest scale and the tone curve's split, because
        // the two have opposite costs. It replaced a toggle and two sliders, one of which
        // (the tone curve's share) had no UI at all — so turning "Background Grain" off
        // used to leave every target dimmed by a mechanism nobody could see.
        it('offers the background grain dial across its whole range', async () => {
            const wrapper = mountSettingsPanel()
            await flushPromises()

            const slider = findSliderByLabel(wrapper, 'Background Grain')
            expect(slider).toBeDefined()
            expect(slider.props('min')).toBe(0.0)
            expect(slider.props('max')).toBe(1.0)

            slider.vm.$emit('update:modelValue', 0.25)
            slider.vm.$emit('change')
            await flushPromises()

            expect(updateSettings).toHaveBeenCalledWith({
                denoise: expect.objectContaining({background_grain: 0.25}),
            })
        })

        // The dial is always available: unlike the toggle it replaced, there is no state
        // in which the control that matters most is hidden.
        it('always shows the grain dial and the structure slider', async () => {
            const wrapper = mountSettingsPanel({
                settings: {
                    denoise: {
                        chroma: false,
                        chroma_strength: 1.0,
                        background_grain: 0.0,
                        luma_strength: 0.0,
                    },
                },
            })
            await flushPromises()

            const labels = wrapper
                .findAllComponents(BaseSlider)
                .map((s) => s.props('label'))
            expect(labels).toContain('Background Grain')
            expect(labels).toContain('Structure strength')
        })

        // Zero is the wavelet's genuine off switch now that the toggle is gone, and 100%
        // is both the tuned value and the ceiling: above it the mottle moves out to a
        // scale the transform cannot reach and stars gain a ring, so the slider must not
        // offer it.
        it('lets structure strength go from off to the tuned value and no further', async () => {
            const wrapper = mountSettingsPanel()
            await flushPromises()

            const slider = findSliderByLabel(wrapper, 'Structure strength')
            expect(slider.props('min')).toBe(0.0)
            expect(slider.props('max')).toBe(1.0)
        })

        // Local contrast ships on, at the setting the plugin's measurements passed on
        // every target, and zero is the picture from before the control existed.
        it('offers Detail from off to its top and sends it with the denoise object', async () => {
            const wrapper = mountSettingsPanel(WITH_DENOISE)
            await flushPromises()

            const slider = findSliderByLabel(wrapper, 'Detail')
            expect(slider.props('min')).toBe(0.0)
            expect(slider.props('max')).toBe(1.0)
            expect(slider.props('modelValue')).toBe(0.5)
            expect(slider.props('disabled')).toBeFalsy()

            slider.vm.$emit('update:modelValue', 0.8)
            slider.vm.$emit('change')
            await flushPromises()

            expect(updateSettings).toHaveBeenCalledWith({
                denoise: expect.objectContaining({detail: 0.8, background_grain: 0.5}),
            })
        })

        // Detail is a gain on the brightness denoiser's planes: with that at zero there is
        // nothing for it to raise, and a live slider that moves nothing is a broken control.
        it('holds Detail while the brightness denoiser is off', async () => {
            const wrapper = mountSettingsPanel({
                ...WITH_DENOISE,
                settings: {denoise: {enabled: true, chroma: true, luma_strength: 0.0, detail: 0.5}},
            })
            await flushPromises()

            const slider = findSliderByLabel(wrapper, 'Detail')
            expect(slider.props('disabled')).toBe(true)
        })

        it('falls back to defaults when the server sends no denoise settings', async () => {
            const wrapper = mountSettingsPanel()
            await flushPromises()

            expect(findToggleByLabel(wrapper, 'Colour Mottle').element.checked).toBe(true)
            const grain = findSliderByLabel(wrapper, 'Background Grain')
            expect(grain.props('modelValue')).toBe(0.5)
            const detail = findSliderByLabel(wrapper, 'Detail')
            expect(detail.props('modelValue')).toBe(0.5)
        })
    })

    describe('AI Denoising', () => {
        const WITH_AI = {
            capabilities: {
                deep_sky: {advanced_rejection: false, rbf_background: false, denoise: true, ai_denoise: true},
            },
        }

        const slider = findSliderByLabel

        it('lives in the Advanced section, locked without the capability', () => {
            const wrapper = mountSettingsPanel()

            const advanced = wrapper
                .findAll('.settings-section')
                .find((section) => section.find('.section-title').text() === 'Advanced')
            expect(advanced?.text()).toContain('AI denoising')
            expect(findToggleByLabel(wrapper, 'AI denoising').attributes('disabled')).toBeDefined()
        })

        it('sends the switch with the rest of the denoise object', async () => {
            const wrapper = mountSettingsPanel(WITH_AI)

            await findToggleByLabel(wrapper, 'AI denoising').setValue(true)
            await flushPromises()

            expect(updateSettings).toHaveBeenCalledWith({
                denoise: expect.objectContaining({ai: true, enabled: true, background_grain: 0.5}),
            })
        })

        // Focus/Finder mode holds the network off without touching the switch, so the
        // observer's choice is there again when the mode ends.
        it('is held, keeping its value, while Denoise is off and in Focus/Finder mode', async () => {
            for (const settings of [
                {denoise: {enabled: false, ai: true}},
                {focus_mode: true, denoise: {ai: true}},
            ]) {
                const wrapper = mountSettingsPanel({...WITH_AI, settings})
                await flushPromises()

                const toggle = findToggleByLabel(wrapper, 'AI denoising')
                expect(toggle.attributes('disabled')).toBeDefined()
                expect(toggle.element.checked).toBe(true)
                expect(wrapper.text()).toContain('Held off')
            }
        })

        it('replaces Structure strength and Detail while it runs, and nothing else', async () => {
            const wrapper = mountSettingsPanel({...WITH_AI, settings: {denoise: {ai: true}}})
            await flushPromises()

            expect(slider(wrapper, 'Structure strength').props('disabled')).toBe(true)
            expect(slider(wrapper, 'Detail').props('disabled')).toBe(true)
            expect(slider(wrapper, 'Background Grain').props('disabled')).toBe(false)
            expect(findToggleByLabel(wrapper, 'Colour Mottle').attributes('disabled')).toBeUndefined()
            expect(wrapper.text()).toContain('replaces Structure strength and Detail')
        })

        it('replaces nothing on a build without the network', async () => {
            const wrapper = mountSettingsPanel({
                capabilities: {deep_sky: {advanced_rejection: false, rbf_background: false, denoise: true}},
                settings: {denoise: {ai: true}},
            })
            await flushPromises()

            expect(slider(wrapper, 'Structure strength').props('disabled')).toBe(false)
            expect(slider(wrapper, 'Detail').props('disabled')).toBe(false)
        })
    })

    describe('Focus/Finder mode', () => {
        // The toggles the mode manages, by their label in this panel. Focus mode owns the
        // snapshot that restores them, so an edit here while it is on would either be
        // reverted on the next toggle or overwrite what the observer chose.
        const MANAGED = [
            'Row/Column Pattern Removal',
            'Colour Mottle',
            'Dithering',
            'Background Subtraction',
            'Shadow Saturation Boost',
        ]

        // The sixth is a slider now: the wavelet's off switch is a zero strength, since
        // the toggle it used to have became part of the Background Grain dial.
        const grainSlider = (wrapper: VueWrapper) =>
            findSliderByLabel(wrapper, 'Structure strength')

        it('holds the brightness denoiser read-only while the mode is on', () => {
            const wrapper = mountSettingsPanel({settings: {focus_mode: true}})

            expect(grainSlider(wrapper).props('disabled')).toBe(true)
        })

        // Held because the mode holds the denoiser it works through — not managed: the
        // mode never snapshots or rewrites the observer's Detail value.
        it('greys Detail out with the brightness denoiser while the mode is on', () => {
            const wrapper = mountSettingsPanel({
                settings: {focus_mode: true},
                capabilities: {deep_sky: {denoise: true}},
            })

            const detail = findSliderByLabel(wrapper, 'Detail')
            expect(detail.props('disabled')).toBe(true)
        })

        // The dial is deliberately *not* managed: it moves the tone curve's split as well
        // as the wavelet, and this mode has no business moving the tone curve.
        it('leaves the Background Grain dial editable while the mode is on', () => {
            const wrapper = mountSettingsPanel({
                settings: {focus_mode: true},
                capabilities: {deep_sky: {denoise: true}},
            })

            const dial = findSliderByLabel(wrapper, 'Background Grain')
            expect(dial.props('disabled')).toBeFalsy()
        })

        it.each(MANAGED)('holds "%s" read-only while the mode is on', (label) => {
            const wrapper = mountSettingsPanel({settings: {focus_mode: true}})

            expect(findToggleByLabel(wrapper, label).attributes('disabled')).toBeDefined()
        })

        it.each(MANAGED)('leaves "%s" editable while the mode is off', (label) => {
            const wrapper = mountSettingsPanel({
                settings: {focus_mode: false},
                capabilities: {
                    deep_sky: {
                        advanced_rejection: false,
                        rbf_background: false,
                        saturation_boost: true,
                        denoise: true,
                    },
                },
            })

            expect(findToggleByLabel(wrapper, label).attributes('disabled')).toBeUndefined()
        })

        // Not in the managed set: it is the *cheap* demosaic, so forcing it either way
        // would trade against the frame rate the mode exists to buy.
        it('leaves Superpixel Debayer editable while the mode is on', () => {
            const wrapper = mountSettingsPanel({settings: {focus_mode: true}})

            expect(
                findToggleByLabel(wrapper, 'Superpixel Debayer').attributes('disabled')
            ).toBeUndefined()
        })

        // Hot pixel rejection has no switch at all: bilinear demosaic turns each hot pixel
        // into a fake star, and Push-To could not plate-solve a single guide frame with the
        // mode holding it off. Only the threshold is the observer's, and the mode leaves it be.
        it('offers no Hot Pixel Rejection switch and keeps its threshold editable', () => {
            const wrapper = mountSettingsPanel({settings: {focus_mode: true}})

            expect(() => findToggleByLabel(wrapper, 'Hot Pixel Rejection')).toThrow()
            const threshold = findSliderByLabel(wrapper, 'Hot Pixel Threshold')
            expect(threshold.exists()).toBe(true)
            expect(threshold.props('disabled')).toBeFalsy()
        })

        it('explains why the settings are greyed out', () => {
            const on = mountSettingsPanel({settings: {focus_mode: true}})
            const off = mountSettingsPanel({settings: {focus_mode: false}})

            expect(on.text()).toContain('held off by Focus/Finder mode')
            expect(off.text()).not.toContain('held off by Focus/Finder mode')
        })

        it('treats a server payload without the field as mode off', () => {
            const wrapper = mountSettingsPanel()

            expect(
                findToggleByLabel(wrapper, 'Background Subtraction').attributes('disabled')
            ).toBeUndefined()
        })
    })
})
