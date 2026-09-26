import {describe, it, expect, vi, beforeEach} from 'vitest'
import {mount} from '@vue/test-utils'
import {nextTick} from 'vue'

vi.mock('../composables/api.js', () => ({
    getAiCompute: vi.fn(),
}))

import HardwareBenchmarkOverlay from './HardwareBenchmarkOverlay.vue'
import {resetAiCompute, useAiCompute} from '../composables/useAiCompute.js'

describe('HardwareBenchmarkOverlay', () => {
    beforeEach(() => resetAiCompute())

    it('shows only while the server is benchmarking', async () => {
        const {report} = useAiCompute()
        const wrapper = mount(HardwareBenchmarkOverlay, {attachTo: document.body})
        expect(wrapper.find('[data-test="benchmark-overlay"]').exists()).toBe(false)

        for (const state of ['checking', 'ready', 'unavailable']) {
            report.value = {state, rungs: []}
            await nextTick()
            expect(wrapper.find('[data-test="benchmark-overlay"]').exists(), state).toBe(false)
        }

        report.value = {state: 'benchmarking', progress: {done: 1, total: 3, testing: 'Integrated GPU — Intel(R) Graphics'}}
        await nextTick()
        const overlay = wrapper.find('[data-test="benchmark-overlay"]')
        expect(overlay.exists()).toBe(true)
        expect(overlay.attributes('role')).toBe('alertdialog')
        expect(overlay.attributes('aria-modal')).toBe('true')
        expect(wrapper.text()).toContain('Benchmarking hardware')
        expect(wrapper.find('[data-test="benchmark-step"]').text()).toBe('Testing Integrated GPU — Intel(R) Graphics (2 of 3)')
        expect(wrapper.find('[role="progressbar"]').attributes('aria-valuenow')).toBe('33')

        report.value = {state: 'ready', rungs: []}
        await nextTick()
        expect(wrapper.find('[data-test="benchmark-overlay"]').exists()).toBe(false)
        wrapper.unmount()
    })

    it('takes focus so the keyboard cannot reach the app behind it', async () => {
        const {report} = useAiCompute()
        report.value = {state: 'benchmarking', progress: {done: 0, total: 2, testing: 'CPU — test'}}
        const wrapper = mount(HardwareBenchmarkOverlay, {attachTo: document.body})
        await nextTick()
        await nextTick()
        expect(document.activeElement).toBe(wrapper.find('[data-test="benchmark-overlay"]').element)
        wrapper.unmount()
    })

    it('says it is finishing once every unit is measured', async () => {
        const {report} = useAiCompute()
        report.value = {state: 'benchmarking', progress: {done: 3, total: 3, testing: null}}
        const wrapper = mount(HardwareBenchmarkOverlay)
        await nextTick()
        expect(wrapper.find('[data-test="benchmark-step"]').text()).toBe('Finishing…')
        expect(wrapper.find('[role="progressbar"]').attributes('aria-valuenow')).toBe('100')
    })
})
