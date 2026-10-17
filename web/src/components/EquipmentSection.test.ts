import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {flushPromises} from '@vue/test-utils'
import type {VueWrapper} from '@vue/test-utils'
import EquipmentSection from './EquipmentSection.vue'
import {updateSettings} from '../composables/api'
import type * as Api from '../composables/api'
import {mountWithDefaults} from '../test/mount'

vi.mock('../composables/api', async (importOriginal) =>
    (await import('../test/mockApi')).mockApi(await importOriginal<typeof Api>()))

describe('EquipmentSection', () => {
    beforeEach(() => vi.useFakeTimers())
    afterEach(() => vi.useRealTimers())

    async function typeFocalLength(wrapper: VueWrapper, value: number) {
        await wrapper.find('input[placeholder="1000"]').setValue(value)
        await vi.advanceTimersByTimeAsync(600)
        await flushPromises()
    }

    it('shows a refused save, even with the section folded', async () => {
        vi.mocked(updateSettings).mockRejectedValueOnce(new Error('Disk full'))
        const wrapper = mountWithDefaults(EquipmentSection)

        await typeFocalLength(wrapper, 800)

        expect(updateSettings).toHaveBeenCalled()
        expect(wrapper.find('.equipment-content').attributes('style')).toContain('display: none')
        expect(wrapper.find('.alert-error').text()).toContain('Disk full')
        expect(wrapper.find('.equipment-content .alert-error').exists()).toBe(false)
    })

    it('clears the error on dismiss and on the next save that lands', async () => {
        vi.mocked(updateSettings).mockRejectedValueOnce(new Error('Disk full'))
        const wrapper = mountWithDefaults(EquipmentSection)
        await typeFocalLength(wrapper, 800)

        await wrapper.find('.alert-error .btn-close').trigger('click')
        expect(wrapper.find('.alert-error').exists()).toBe(false)

        vi.mocked(updateSettings).mockRejectedValueOnce(new Error('Disk full'))
        await typeFocalLength(wrapper, 900)
        expect(wrapper.find('.alert-error').exists()).toBe(true)

        await typeFocalLength(wrapper, 1000)
        expect(wrapper.find('.alert-error').exists()).toBe(false)
    })

    // The header folds on click; the chevron inside it only shows the state, or a click on it folded twice.
    it('folds once per click on the header chevron or the header itself', async () => {
        const wrapper = mountWithDefaults(EquipmentSection)
        const content = () => wrapper.find('.equipment-content').attributes('style') ?? ''

        await wrapper.find('.equipment-header .collapse-toggle').trigger('click')
        expect(content()).not.toContain('display: none')

        await wrapper.find('.equipment-header h3').trigger('click')
        expect(content()).toContain('display: none')

        await wrapper.find('.manual-sensor-header .collapse-toggle').trigger('click')
        expect(wrapper.find('.manual-sensor-content').attributes('style') ?? '').not.toContain('display: none')
    })

    // The manual-specs switch sits in the header that folds them: a click on it is the switch's only.
    it('switches manual specs off without folding them', async () => {
        const wrapper = mountWithDefaults(EquipmentSection)
        const specs = () => wrapper.find('.manual-sensor-content').attributes('style') ?? ''
        const manual = wrapper.find<HTMLInputElement>('.manual-sensor-header input[type="checkbox"]')
        // A browser's click flips the box, then reports the change; a detached jsdom one only flips it.
        const click = async () => {
            await manual.trigger('click')
            await manual.trigger('change')
        }

        await click()
        expect(manual.element.checked).toBe(true)
        expect(specs()).not.toContain('display: none')

        await click()
        expect(manual.element.checked).toBe(false)
        expect(specs()).not.toContain('display: none')
    })
})
