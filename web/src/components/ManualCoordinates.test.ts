import {describe, it, expect, expectTypeOf, vi} from 'vitest'
import {mount, flushPromises} from '@vue/test-utils'
import type {ComponentMountingOptions} from '@vue/test-utils'
import ManualCoordinates from './ManualCoordinates.vue'
import type {TypedCoordinates} from '../composables/useCoordinates.js'

async function mountOpen(submit: (coords: TypedCoordinates) => Promise<boolean>) {
    const wrapper = mount(ManualCoordinates, {props: {submit}})
    await wrapper.findComponent({name: 'BaseToggle'}).vm.$emit('update:modelValue', true)
    const [ra, dec] = wrapper.findAll<HTMLInputElement>('.coord-input')
    await ra.setValue('10.68')
    await dec.setValue('41.27')
    return {wrapper, ra, dec}
}

describe('ManualCoordinates', () => {
    it('submits the parsed coordinates and clears the fields once the target took', async () => {
        const submit = vi.fn(async () => true)
        const {wrapper, ra, dec} = await mountOpen(submit)

        await wrapper.find('.set-coords-btn').trigger('click')
        await flushPromises()

        expect(submit).toHaveBeenCalledWith({ra: 10.68, dec: 41.27})
        expect(ra.element.value).toBe('')
        expect(dec.element.value).toBe('')
    })

    it('keeps what was typed when the target was refused', async () => {
        const {wrapper, ra} = await mountOpen(vi.fn(async () => false))

        await wrapper.find('.set-coords-btn').trigger('click')
        await flushPromises()

        expect(ra.element.value).toBe('10.68')
    })

    it('does not submit coordinates that do not parse', async () => {
        const submit = vi.fn(async () => true)
        const {wrapper, ra} = await mountOpen(submit)
        await ra.setValue('not a coordinate')

        await wrapper.find('.set-coords-btn').trigger('click')

        expect(submit).not.toHaveBeenCalled()
        expect(wrapper.find('.coord-error').exists()).toBe(true)
    })

    it('takes a submit that says whether the target took', () => {
        type Submit = NonNullable<ComponentMountingOptions<typeof ManualCoordinates>['props']>['submit']
        expectTypeOf<Submit>().toEqualTypeOf<(coords: TypedCoordinates) => Promise<boolean>>()
        // @ts-expect-error -- the fields clear on a `true` alone, so the answer must say whether it took
        expect(mount(ManualCoordinates, {props: {submit: async () => undefined}}).exists()).toBe(true)
    })
})
