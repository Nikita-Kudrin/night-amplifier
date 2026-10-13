/** The AI compute selector (`AiComputeSettings`), which awaits the panel's save before refetching the report. */
import {describe, it, expect, vi, beforeEach} from 'vitest'
import {flushPromises} from '@vue/test-utils'
import {mountSettingsPanel} from '../test/settingsPanel.js'
import {__resetForTests as resetAiCompute, useAiCompute} from '../composables/useAiCompute.js'

vi.mock('../composables/api.js', async (importOriginal) =>
    (await import('../test/mockApi.js')).mockApi(await importOriginal(), {
        updateSettings: vi.fn(),
        getAiCompute: vi.fn(),
        remeasureAiCompute: vi.fn(),
    }))

import {getAiCompute, remeasureAiCompute, updateSettings} from '../composables/api.js'

describe('SettingsPanel AI compute', () => {
    beforeEach(() => {
        vi.clearAllMocks()
        updateSettings.mockResolvedValue({})
    })

    describe('AI Compute', () => {
        const WITH_AI = {
            capabilities: {
                deep_sky: {advanced_rejection: false, rbf_background: false, denoise: true, ai_denoise: true},
            },
        }

        /** This laptop: Iris Xe measured, the NVIDIA card without a driver, no NPU. */
        const LAPTOP = {
            generation: 3,
            state: 'ready',
            auto: 'integrated_gpu',
            effective: 'integrated_gpu',
            notice: null,
            rungs: [
                {rung: 'npu', usable: false, device: null, reason: 'No supported NPU found'},
                {
                    rung: 'discrete_gpu',
                    usable: false,
                    device: 'NVIDIA [10de:25ba]',
                    reason: 'present, but no Vulkan driver (kernel driver: nouveau)',
                },
                {rung: 'integrated_gpu', usable: true, device: 'Intel Iris Xe', api: 'Vulkan', precision: 'f32', ms_per_frame: 64.7},
                {rung: 'cpu', usable: true, device: 'i9-12900H', api: 'engine', precision: 'f32', ms_per_frame: 219.3},
            ],
        }

        const select = (wrapper) => wrapper.find('[data-test="ai-compute-select"]')

        beforeEach(() => {
            resetAiCompute()
            getAiCompute.mockResolvedValue(LAPTOP)
        })

        it('offers every rung and greys out what this computer cannot use, with the reason', async () => {
            useAiCompute().report.value = LAPTOP
            const wrapper = mountSettingsPanel(WITH_AI)
            await flushPromises()

            const options = select(wrapper).findAll('option')
            expect(options.map((o) => o.attributes('value'))).toEqual(['auto', 'npu', 'discrete_gpu', 'integrated_gpu', 'cpu'])
            expect(options[0].text()).toBe('Auto — Integrated GPU · 65 ms')
            expect(options[1].attributes('disabled')).toBeDefined()
            expect(options[2].attributes('disabled')).toBeDefined()
            expect(options[3].attributes('disabled')).toBeUndefined()
            expect(select(wrapper).attributes('disabled')).toBeUndefined()

            const reasons = wrapper.find('[data-test="ai-compute-reasons"]')
            expect(reasons.text()).toContain('NVIDIA [10de:25ba] — present, but no Vulkan driver')
            expect(reasons.find('a').attributes('href')).toBe('/night-amplifier/system-dependencies')
            expect(wrapper.find('[data-test="ai-compute-hint"]').text()).toContain('Runs on the Integrated GPU: Intel Iris Xe')
        })

        it('saves the choice with the rest of the denoise object and refetches the report', async () => {
            useAiCompute().report.value = LAPTOP
            const wrapper = mountSettingsPanel(WITH_AI)
            await flushPromises()

            await select(wrapper).setValue('cpu')
            await flushPromises()

            expect(updateSettings).toHaveBeenCalledWith({
                denoise: expect.objectContaining({ai_compute: 'cpu', enabled: true, ai: false}),
            })
            expect(getAiCompute).toHaveBeenCalled()
        })

        it('shows the server’s notice when a forced rung is not available here', async () => {
            const notice = 'NPU is not available on this computer; using Auto (Integrated GPU).'
            useAiCompute().report.value = {...LAPTOP, notice}
            const wrapper = mountSettingsPanel({...WITH_AI, settings: {denoise: {ai_compute: 'npu'}}})
            await flushPromises()
            expect(wrapper.find('[data-test="ai-compute-hint"]').text()).toBe(notice)
            expect(select(wrapper).element.value).toBe('npu')
        })

        it('waits for the benchmark, and stays usable while the AI switch is off', async () => {
            useAiCompute().report.value = {state: 'benchmarking', rungs: []}
            const wrapper = mountSettingsPanel({...WITH_AI, settings: {denoise: {ai: false}}})
            await flushPromises()
            expect(select(wrapper).attributes('disabled')).toBeDefined()
            expect(select(wrapper).find('option').text()).toBe('Auto (checking hardware…)')

            useAiCompute().report.value = LAPTOP
            await flushPromises()
            expect(select(wrapper).attributes('disabled')).toBeUndefined()
        })

        it('is locked without the capability', async () => {
            useAiCompute().report.value = LAPTOP
            const wrapper = mountSettingsPanel()
            await flushPromises()
            expect(select(wrapper).attributes('disabled')).toBeDefined()
            expect(wrapper.find('[data-test="ai-compute-reasons"]').exists()).toBe(false)
        })

        describe('Measure again', () => {
            const button = (wrapper) => wrapper.find('[data-test="ai-compute-remeasure"]')

            it('asks the server to measure again and reads the new report', async () => {
                useAiCompute().report.value = LAPTOP
                remeasureAiCompute.mockResolvedValue({...LAPTOP, state: 'checking'})
                getAiCompute.mockResolvedValue({...LAPTOP, state: 'checking'})
                const wrapper = mountSettingsPanel(WITH_AI)
                await flushPromises()

                await button(wrapper).trigger('click')
                await flushPromises()
                expect(remeasureAiCompute).toHaveBeenCalledTimes(1)
                expect(getAiCompute).toHaveBeenCalled()
                expect(useAiCompute().report.value.state).toBe('checking')
            })

            it('shows why the server refused', async () => {
                useAiCompute().report.value = LAPTOP
                remeasureAiCompute.mockRejectedValue(new Error('Stop the capture to measure the hardware again'))
                const wrapper = mountSettingsPanel(WITH_AI)
                await flushPromises()

                await button(wrapper).trigger('click')
                await flushPromises()
                expect(wrapper.text()).toContain('Stop the capture to measure the hardware again')
                expect(button(wrapper).attributes('disabled')).toBeUndefined()
            })

            it('is offered only once there is a result, and only with the network', async () => {
                useAiCompute().report.value = {state: 'benchmarking', rungs: []}
                const measuring = mountSettingsPanel(WITH_AI)
                await flushPromises()
                expect(button(measuring).exists()).toBe(false)

                useAiCompute().report.value = LAPTOP
                const locked = mountSettingsPanel()
                await flushPromises()
                expect(button(locked).exists()).toBe(false)
            })
        })
    })
})
