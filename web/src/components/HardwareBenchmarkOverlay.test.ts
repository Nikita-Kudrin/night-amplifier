import {describe, it, expect, vi, beforeEach} from 'vitest'
import {mount} from '@vue/test-utils'
import {nextTick} from 'vue'

vi.mock('../composables/api.js', async (importOriginal) =>
    (await import('../test/mockApi.js')).mockApi(await importOriginal(), {
        getAiCompute: vi.fn(),
    }))

import HardwareBenchmarkOverlay from './HardwareBenchmarkOverlay.vue'
import {__resetForTests as resetAiCompute, useAiCompute} from '../composables/useAiCompute.js'
import {aiComputeReport} from '../test/fixtures.js'
import type {BenchmarkState} from '../composables/api.types.js'

describe('HardwareBenchmarkOverlay', () => {
    beforeEach(() => resetAiCompute())

    it('shows only while the server is benchmarking', async () => {
        const {report} = useAiCompute()
        const wrapper = mount(HardwareBenchmarkOverlay, {attachTo: document.body})
        expect(wrapper.find('[data-test="benchmark-overlay"]').exists()).toBe(false)

        for (const state of ['checking', 'ready', 'unavailable'] satisfies BenchmarkState[]) {
            report.value = aiComputeReport({state})
            await nextTick()
            expect(wrapper.find('[data-test="benchmark-overlay"]').exists(), state).toBe(false)
        }

        report.value = aiComputeReport({
            state: 'benchmarking',
            progress: {done: 1, total: 3, testing: 'Integrated GPU — Intel(R) Graphics'},
        })
        await nextTick()
        const overlay = wrapper.find('[data-test="benchmark-overlay"]')
        expect(overlay.exists()).toBe(true)
        expect(overlay.attributes('role')).toBe('alertdialog')
        expect(overlay.attributes('aria-modal')).toBe('true')
        expect(wrapper.text()).toContain('Benchmarking hardware')
        expect(wrapper.find('.progress-text').text()).toBe('Testing Integrated GPU — Intel(R) Graphics (2 of 3)')
        expect(wrapper.find('[role="progressbar"]').attributes('aria-valuenow')).toBe('33')

        report.value = aiComputeReport({state: 'ready'})
        await nextTick()
        expect(wrapper.find('[data-test="benchmark-overlay"]').exists()).toBe(false)
        wrapper.unmount()
    })

    it('takes focus so the keyboard cannot reach the app behind it', async () => {
        const {report} = useAiCompute()
        report.value = aiComputeReport({state: 'benchmarking', progress: {done: 0, total: 2, testing: 'CPU — test'}})
        const wrapper = mount(HardwareBenchmarkOverlay, {attachTo: document.body})
        await nextTick()
        await nextTick()
        expect(document.activeElement).toBe(wrapper.find('[data-test="benchmark-overlay"]').element)
        wrapper.unmount()
    })

    it('says it is finishing once every unit is measured', async () => {
        const {report} = useAiCompute()
        report.value = aiComputeReport({state: 'benchmarking', progress: {done: 3, total: 3, testing: null}})
        const wrapper = mount(HardwareBenchmarkOverlay)
        await nextTick()
        expect(wrapper.find('.progress-text').text()).toBe('Finishing…')
        expect(wrapper.find('[role="progressbar"]').attributes('aria-valuenow')).toBe('100')
    })
})
