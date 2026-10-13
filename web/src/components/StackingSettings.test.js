import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {mount} from '@vue/test-utils'
import StackingSettings from './StackingSettings.vue'

const PROPS = {
    weightingPreset: 'balanced',
    rejectionMethod: 'SigmaClip',
    rejectionSigma: 3.0,
    planetaryAutoTracking: true,
    planetaryMultiPointAlignment: false,
    advancedRejectionAvailable: true,
}

function sigmaSlider(wrapper) {
    return wrapper.findAllComponents({name: 'BaseSlider'}).find((s) => s.props('label') === 'Rejection Sigma')
}

async function dragSigma(wrapper, value) {
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
        const options = wrapper.findAll('#rejection-method-select option')
        const locked = options.filter((o) => o.element.disabled).map((o) => o.text())

        expect(locked.length).toBeGreaterThan(0)
        expect(locked.every((text) => text.endsWith('🔒'))).toBe(true)
        expect(wrapper.findComponent({name: 'BaseProLock'}).exists()).toBe(true)

        await wrapper.find('#weighting-preset-select').setValue('nebulae')
        expect(wrapper.emitted('apply')).toEqual([['weighting_preset', 'nebulae']])
    })
})
