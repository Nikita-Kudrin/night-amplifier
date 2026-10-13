import {mount} from '@vue/test-utils'
import {nextTick} from 'vue'
import ExposureControl from './ExposureControl.vue'

function mountControl(props = {}) {
    return mount(ExposureControl, {
        props: {modelValue: 500000, 'onUpdate:modelValue': vi.fn(), ...props},
    })
}

const field = (wrapper) => wrapper.find('input[type="number"]')
const unit = (wrapper) => wrapper.find('select')

describe('ExposureControl', () => {
    it('shows the exposure in the largest unit it fills', () => {
        const wrapper = mountControl({modelValue: 2000000})

        expect(field(wrapper).element.value).toBe('2')
        expect(unit(wrapper).element.value).toBe('s')
    })

    it('keeps the model current while typing, before any change event', async () => {
        const onUpdate = vi.fn()
        const wrapper = mountControl({'onUpdate:modelValue': onUpdate})

        field(wrapper).element.value = '750'
        await field(wrapper).trigger('input')
        await nextTick()

        expect(onUpdate).toHaveBeenLastCalledWith(750000)
        expect(wrapper.emitted('apply')).toBeUndefined()
    })

    it('commits on change, in whole microseconds', async () => {
        const wrapper = mountControl({modelValue: 2000000})

        await field(wrapper).setValue(4.1)

        expect(wrapper.emitted('apply')).toEqual([[4100000]])
    })

    it('commits a preset and highlights the active one', async () => {
        const wrapper = mountControl({modelValue: 40000, activeUs: 40000})

        const active = wrapper.find('.btn-preset.active')
        expect(active.text()).toBe('40')
        await wrapper.findAll('.btn-preset').find((b) => b.text() === '100').trigger('click')

        expect(wrapper.emitted('apply')).toEqual([[100000]])
    })

    it('keeps the unit chosen when the same exposure comes back', async () => {
        const wrapper = mountControl({modelValue: 500000})
        await unit(wrapper).setValue('s')
        await field(wrapper).setValue(0.5)

        await wrapper.setProps({modelValue: 500000})

        expect(unit(wrapper).element.value).toBe('s')
        expect(field(wrapper).element.value).toBe('0.5')
    })

    it('shows a different exposure from outside in its own unit', async () => {
        const wrapper = mountControl({modelValue: 500000})

        await wrapper.setProps({modelValue: 300})

        expect(field(wrapper).element.value).toBe('300')
        expect(unit(wrapper).element.value).toBe('us')
    })

    // An emptied field read as exposure 0, committed on change and sent by Start.
    it('keeps the model and commits nothing for an emptied field, then shows the exposure again', async () => {
        const onUpdate = vi.fn()
        const wrapper = mountControl({modelValue: 2000000, 'onUpdate:modelValue': onUpdate})

        await field(wrapper).setValue('')
        await nextTick()

        expect(onUpdate).not.toHaveBeenCalled()
        expect(wrapper.emitted('apply')).toBeUndefined()
        expect(field(wrapper).element.value).toBe('2')
        expect(unit(wrapper).element.value).toBe('s')
    })

    it('commits nothing for a zero exposure', async () => {
        const wrapper = mountControl()

        await field(wrapper).setValue(0)

        expect(wrapper.emitted('apply')).toBeUndefined()
        expect(field(wrapper).element.value).toBe('500')
    })
})
