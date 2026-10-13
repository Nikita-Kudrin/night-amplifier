import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {flushPromises} from '@vue/test-utils'
import EquipmentSection from './EquipmentSection.vue'
import {updateSettings} from '../composables/api.js'
import {mountWithDefaults} from '../test/mount.js'

vi.mock('../composables/api.js', async (importOriginal) =>
    (await import('../test/mockApi.js')).mockApi(await importOriginal()))

describe('EquipmentSection', () => {
    beforeEach(() => vi.useFakeTimers())
    afterEach(() => vi.useRealTimers())

    async function typeFocalLength(wrapper, value) {
        await wrapper.find('input[placeholder="1000"]').setValue(value)
        await vi.advanceTimersByTimeAsync(600)
        await flushPromises()
    }

    it('shows a refused save, even with the section folded', async () => {
        updateSettings.mockRejectedValueOnce(new Error('Disk full'))
        const wrapper = mountWithDefaults(EquipmentSection)

        await typeFocalLength(wrapper, 800)

        expect(updateSettings).toHaveBeenCalled()
        expect(wrapper.find('.equipment-content').attributes('style')).toContain('display: none')
        expect(wrapper.find('.alert-error').text()).toContain('Disk full')
        expect(wrapper.find('.equipment-content .alert-error').exists()).toBe(false)
    })

    it('clears the error on dismiss and on the next save that lands', async () => {
        updateSettings.mockRejectedValueOnce(new Error('Disk full'))
        const wrapper = mountWithDefaults(EquipmentSection)
        await typeFocalLength(wrapper, 800)

        await wrapper.find('.alert-error .btn-close').trigger('click')
        expect(wrapper.find('.alert-error').exists()).toBe(false)

        updateSettings.mockRejectedValueOnce(new Error('Disk full'))
        await typeFocalLength(wrapper, 900)
        expect(wrapper.find('.alert-error').exists()).toBe(true)

        await typeFocalLength(wrapper, 1000)
        expect(wrapper.find('.alert-error').exists()).toBe(false)
    })
})
