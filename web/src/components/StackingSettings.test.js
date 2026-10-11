import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {mount} from '@vue/test-utils'
import {h} from 'vue'
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

    it('sends the sigma once, 300 ms after the last drag', async () => {
        const wrapper = mount(StackingSettings, {props: PROPS})

        await dragSigma(wrapper, 2.5)
        await dragSigma(wrapper, 2.2)
        expect(wrapper.emitted('apply')).toBeUndefined()

        vi.advanceTimersByTime(300)
        expect(wrapper.emitted('apply')).toEqual([['rejection_sigma', 2.2]])
    })

    // Vue drops an emit from an unmounted component, so a sigma dragged just before the
    // Settings tab closed never reached the server.
    it('sends a pending sigma when it unmounts', async () => {
        const onApply = vi.fn()
        const wrapper = mount({render: () => h(StackingSettings, {...PROPS, onApply})})

        await dragSigma(wrapper.findComponent(StackingSettings), 2.5)
        wrapper.unmount()

        expect(onApply).toHaveBeenCalledTimes(1)
        expect(onApply).toHaveBeenCalledWith('rejection_sigma', 2.5)
        vi.advanceTimersByTime(300)
        expect(onApply).toHaveBeenCalledTimes(1)
    })
})
