import {mount} from '@vue/test-utils'
import BaseSlider from './BaseSlider.vue'

describe('BaseSlider', () => {
    function mountSlider(props = {}) {
        return mount(BaseSlider, {
            props: {modelValue: 46, label: 'Gain', min: 0, max: 500, ...props},
        })
    }

    describe('value display', () => {
        it('shows the read-only value in the label without a number input', () => {
            const wrapper = mountSlider()

            expect(wrapper.find('.current-value').text()).toBe('46')
            expect(wrapper.find('input[type="number"]').exists()).toBe(false)
        })

        // Two copies of the same number a few pixels apart read as two settings.
        it('shows the value exactly once when the number input is present', () => {
            const wrapper = mountSlider({showInput: true})

            expect(wrapper.find('.current-value').exists()).toBe(false)
            expect(wrapper.find('input[type="number"]').element.value).toBe('46')
        })

        it('applies formatValue to the read-only value', () => {
            const wrapper = mountSlider({formatValue: (v) => `${v}%`})

            expect(wrapper.find('.current-value').text()).toBe('46%')
        })
    })

    describe('emits', () => {
        it('emits update:modelValue while the slider is dragged', async () => {
            const wrapper = mountSlider()

            await wrapper.find('input[type="range"]').setValue(120)

            expect(wrapper.emitted('update:modelValue')[0]).toEqual([120])
        })

        it('emits change from the number input as a number, not a string', async () => {
            const wrapper = mountSlider({showInput: true})
            const input = wrapper.find('input[type="number"]')

            input.element.value = '200'
            await input.trigger('change')

            expect(wrapper.emitted('change')[0]).toEqual([200])
        })
    })
})
