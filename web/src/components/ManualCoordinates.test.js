import {describe, it, expect, vi} from 'vitest'
import {mount, flushPromises} from '@vue/test-utils'
import ManualCoordinates from './ManualCoordinates.vue'

async function mountOpen(submit) {
    const wrapper = mount(ManualCoordinates, {props: {submit}})
    await wrapper.findComponent({name: 'BaseToggle'}).vm.$emit('update:modelValue', true)
    const [ra, dec] = wrapper.findAll('.coord-input')
    await ra.setValue('10.68')
    await dec.setValue('41.27')
    return {wrapper, ra, dec}
}

describe('ManualCoordinates', () => {
    it('submits the parsed coordinates and clears the fields once the target took', async () => {
        const submit = vi.fn().mockResolvedValue(true)
        const {wrapper, ra, dec} = await mountOpen(submit)

        await wrapper.find('.set-coords-btn').trigger('click')
        await flushPromises()

        expect(submit).toHaveBeenCalledWith({ra: 10.68, dec: 41.27})
        expect(ra.element.value).toBe('')
        expect(dec.element.value).toBe('')
    })

    it('keeps what was typed when the target was refused', async () => {
        const {wrapper, ra} = await mountOpen(vi.fn().mockResolvedValue(undefined))

        await wrapper.find('.set-coords-btn').trigger('click')
        await flushPromises()

        expect(ra.element.value).toBe('10.68')
    })

    it('does not submit coordinates that do not parse', async () => {
        const submit = vi.fn()
        const {wrapper, ra} = await mountOpen(submit)
        await ra.setValue('not a coordinate')

        await wrapper.find('.set-coords-btn').trigger('click')

        expect(submit).not.toHaveBeenCalled()
        expect(wrapper.find('.coord-error').exists()).toBe(true)
    })
})
