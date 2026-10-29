import {mount} from '@vue/test-utils'
import type {ComponentMountingOptions, VueWrapper} from '@vue/test-utils'
import {nextTick} from 'vue'
import ProcessingSettings from './ProcessingSettings.vue'
import {settings} from '../test/fixtures'
import type {SettingsOverrides} from '../test/fixtures'
import type {SettingApply} from '../utils/settings'
import {findSliderByLabel as slider, findToggleByLabel} from '../test/settingsPanel'
import {HELP_TEXTS} from '../constants'

type Props = ComponentMountingOptions<typeof ProcessingSettings>['props']

function mountSection(props: Partial<Props> = {}, server: SettingsOverrides = {}) {
    return mount(ProcessingSettings, {
        props: {settings: settings(server), rbfBackgroundAvailable: true, saturationBoostAvailable: true, ...props},
        global: {stubs: {BaseProLock: true}},
    })
}

async function release(wrapper: VueWrapper, label: string, value: number) {
    slider(wrapper, label).vm.$emit('update:modelValue', value)
    slider(wrapper, label).vm.$emit('change', value)
    await nextTick()
}

describe('ProcessingSettings', () => {
    afterEach(() => vi.useRealTimers())

    // It used to wait 300 ms; every settings slider now sends on release.
    it('sends the saturation strength on release, at once', async () => {
        vi.useFakeTimers()
        const wrapper = mountSection({}, {saturation_boost: true})

        await release(wrapper, 'Saturation Strength', 0.7)

        expect(wrapper.emitted<SettingApply>('apply')).toEqual([['saturation_boost_strength', 0.7]])
    })

    // Eyepiece edits the same group, so only the changed field goes; the panel lays it over
    // its own copy (`SettingsPanel.applyEyepiece`).
    it('hands over only the tone field it changed', async () => {
        const wrapper = mountSection({}, {eyepiece: {binoview: false, intensity: 0.3, dither: false}})

        await release(wrapper, 'Black level', 0.4)
        await findToggleByLabel(wrapper, 'Dithering').setValue(true)

        expect(wrapper.emitted('apply')).toEqual([
            ['eyepiece', {intensity: 0.4}],
            ['eyepiece', {dither: true}],
        ])
    })

    it('follows a tone edit made elsewhere', async () => {
        const wrapper = mountSection({}, {eyepiece: {black_floor: 0.02}})

        await wrapper.setProps({settings: settings({eyepiece: {black_floor: -0.03}})})

        expect(slider(wrapper, 'Black floor').props('modelValue')).toBe(-0.03)
    })

    it('keeps its edits off the panel\'s copy until it emits them', async () => {
        const mirror = settings()
        const wrapper = mountSection({settings: mirror})

        slider(wrapper, 'Black level').vm.$emit('update:modelValue', 0.9)
        await nextTick()

        expect(mirror.eyepiece.intensity).not.toBe(0.9)
    })

    it('locks the RBF algorithm without the capability', () => {
        const wrapper = mountSection({rbfBackgroundAvailable: false}, {background_subtraction: true})
        const rbf = wrapper.findAll<HTMLOptionElement>('#bg-algorithm-select option').find((o) => o.element.disabled)

        expect(rbf?.text()).toContain('🔒')
    })

    // A locked toggle cannot be switched off, so a build without the boost must never show
    // it on: a fresh Community install reads the server's defaults.
    it('never shows the locked boost on, with the server\'s defaults', () => {
        const wrapper = mountSection({saturationBoostAvailable: false})

        expect(findToggleByLabel(wrapper, 'Shadow Saturation Boost').element.disabled).toBe(true)
        expect(findToggleByLabel(wrapper, 'Shadow Saturation Boost').element.checked).toBe(false)
    })

    // Planetary renders through its own levels; the choices stay the observer's for Deep Sky.
    it('holds what Planetary does not use, and keeps the display controls it does', () => {
        const wrapper = mountSection({planetary: true}, {background_subtraction: true, saturation_boost: true})

        expect(wrapper.text()).toContain(HELP_TEXTS.planetary_held)
        for (const label of ['Background Subtraction', 'Shadow Saturation Boost']) {
            expect(findToggleByLabel(wrapper, label).element.disabled, label).toBe(true)
            expect(findToggleByLabel(wrapper, label).element.checked, label).toBe(true)
        }
        for (const label of ['Color Intensity', 'Black level', 'Saturation Strength']) {
            expect(slider(wrapper, label).props('disabled'), label).toBe(true)
        }
        expect(wrapper.find<HTMLSelectElement>('#bg-algorithm-select').element.disabled).toBe(true)
        expect(slider(wrapper, 'Black floor').props('disabled')).toBeFalsy()
        expect(findToggleByLabel(wrapper, 'Dithering').element.disabled).toBe(false)
        expect(wrapper.emitted('apply')).toBeUndefined()
    })

    it('holds nothing for Deep Sky', () => {
        const wrapper = mountSection({planetary: false}, {eyepiece: {black_floor: -0.03}})

        expect(wrapper.text()).not.toContain(HELP_TEXTS.planetary_held)
        expect(findToggleByLabel(wrapper, 'Background Subtraction').element.disabled).toBe(false)
        expect(slider(wrapper, 'Color Intensity').props('disabled')).toBeFalsy()
        expect(findToggleByLabel(wrapper, 'Darker sky').element.disabled).toBe(false)
    })

    // Darker sky shapes Black floor's darkening half, which Planetary's black sky never runs.
    it('holds Darker sky for Planetary even with Black floor darkening', () => {
        const wrapper = mountSection({planetary: true}, {eyepiece: {black_floor: -0.03, darker_sky: true}})

        expect(findToggleByLabel(wrapper, 'Darker sky').element.disabled).toBe(true)
        expect(findToggleByLabel(wrapper, 'Darker sky').element.checked).toBe(true)
        expect(slider(wrapper, 'Black floor').props('disabled')).toBeFalsy()
    })

    it('holds dithering, background subtraction and the boost while Focus/Finder mode is on', () => {
        const wrapper = mountSection({focusMode: true})

        expect(wrapper.text()).toContain(HELP_TEXTS.focus_mode_held)
        for (const label of ['Dithering', 'Background Subtraction', 'Shadow Saturation Boost']) {
            expect(findToggleByLabel(wrapper, label).element.disabled, label).toBe(true)
        }
    })
})
