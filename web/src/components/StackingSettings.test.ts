import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {mount} from '@vue/test-utils'
import type {ComponentMountingOptions, VueWrapper} from '@vue/test-utils'
import StackingSettings from './StackingSettings.vue'
import {findSliderByLabel} from '../test/settingsPanel'
import type {SettingApply} from '../utils/settings'

const PROPS = {
    weightingPreset: 'balanced',
    rejectionMethod: 'SigmaClip',
    rejectionSigma: 3.0,
    planetaryAutoTracking: true,
    planetaryMultiPointAlignment: false,
    advancedRejectionAvailable: true,
} satisfies ComponentMountingOptions<typeof StackingSettings>['props']

function sigmaSlider(wrapper: VueWrapper) {
    return findSliderByLabel(wrapper, 'Rejection Sigma')
}

async function dragSigma(wrapper: VueWrapper, value: number) {
    const slider = sigmaSlider(wrapper)
    slider.vm.$emit('update:modelValue', value)
    slider.vm.$emit('change', value)
    await wrapper.vm.$nextTick()
}

describe('StackingSettings', () => {
    beforeEach(() => vi.useFakeTimers())
    afterEach(() => vi.useRealTimers())

    // One rule for every settings slider: a release is a decision, sent at once; only a
    // typed number waits for the typing to stop. The sigma used to wait 300 ms.
    it('sends the sigma on each release, at once', async () => {
        const wrapper = mount(StackingSettings, {props: PROPS})

        await dragSigma(wrapper, 2.5)
        expect(wrapper.emitted('apply')).toEqual([['rejection_sigma', 2.5]])
        await dragSigma(wrapper, 2.2)

        expect(wrapper.emitted('apply')).toEqual([['rejection_sigma', 2.5], ['rejection_sigma', 2.2]])
    })

    it('offers the Pro rejection methods locked, and sends a choice', async () => {
        const wrapper = mount(StackingSettings, {
            props: {...PROPS, advancedRejectionAvailable: false},
            global: {stubs: {BaseProLock: true}},
        })
        const options = wrapper.findAll<HTMLOptionElement>('#rejection-method-select option')
        const locked = options.filter((o) => o.element.disabled).map((o) => o.text())

        expect(locked.length).toBeGreaterThan(0)
        expect(locked.every((text) => text.endsWith('🔒'))).toBe(true)
        expect(wrapper.findComponent({name: 'BaseProLock'}).exists()).toBe(true)

        await wrapper.find('#weighting-preset-select').setValue('nebulae')
        expect(wrapper.emitted<SettingApply>('apply')).toEqual([['weighting_preset', 'nebulae']])
    })

    // A section's `apply` pairs each key with its own value's type (`SettingApply`).
    it('emits a key only with a value of its own type', () => {
        type OnApply = NonNullable<NonNullable<ComponentMountingOptions<typeof StackingSettings>['props']>['onApply']>
        expectTypeOf<Parameters<OnApply>>().toEqualTypeOf<SettingApply>()
        expectTypeOf<['rejection_sigma', number]>().toExtend<SettingApply>()
        expectTypeOf<['rejection_sigma', string]>().not.toExtend<SettingApply>()
        expectTypeOf<['weighting_preset', 'sharp']>().not.toExtend<SettingApply>()
        expectTypeOf<['rejection_sigmas', number]>().not.toExtend<SettingApply>()
    })
})
