import {describe, it, expect} from 'vitest'
import {aiComputeOptions, aiComputeSummary, unusableRungs, SYSTEM_DEPENDENCIES_URL} from './aiCompute'
import type {AiComputeReport} from '../composables/api.types'
import {aiComputeReport} from '../test/fixtures'

/** This laptop's report: Iris Xe usable, the NVIDIA card without a Vulkan driver. */
function laptop(overrides: Partial<AiComputeReport> = {}): AiComputeReport {
    return {
        generation: 7,
        state: 'ready',
        progress: null,
        auto: 'integrated_gpu',
        effective: 'integrated_gpu',
        notice: null,
        rungs: [
            {rung: 'npu', usable: false, device: null, api: null, precision: null, ms_per_frame: null, reason: 'No supported NPU found'},
            {
                rung: 'discrete_gpu',
                usable: false,
                device: 'NVIDIA [10de:25ba]',
                api: null,
                precision: null,
                ms_per_frame: null,
                reason: 'present, but no Vulkan driver (kernel driver: nouveau)',
            },
            {
                rung: 'integrated_gpu',
                usable: true,
                device: 'Intel(R) Graphics (ADL GT2)',
                api: 'Vulkan · Mesa 23.2.1',
                precision: 'f32',
                ms_per_frame: 64.7,
                reason: null,
            },
            {
                rung: 'cpu',
                usable: true,
                device: 'i9-12900H, 20 threads',
                api: 'engine',
                precision: 'f32',
                ms_per_frame: 219.3,
                reason: null,
            },
        ],
        ...overrides,
    }
}

describe('aiComputeOptions', () => {
    it('lists Auto with its pick, then every rung, greying out what this computer cannot use', () => {
        const options = aiComputeOptions(laptop())
        expect(options.map((o) => o.value)).toEqual(['auto', 'npu', 'discrete_gpu', 'integrated_gpu', 'cpu'])
        expect(options[0].label).toBe('Auto — Integrated GPU · 65 ms')
        expect(options[1]).toMatchObject({disabled: true, label: 'NPU — not available', reason: 'No supported NPU found'})
        expect(options[2].disabled).toBe(true)
        expect(options[3]).toMatchObject({disabled: false, label: 'Integrated GPU — Intel(R) Graphics (ADL GT2) · 65 ms'})
        expect(options[4].disabled).toBe(false)
    })

    it('greys nothing out before the benchmark has an answer', () => {
        for (const state of ['checking', 'benchmarking', 'unavailable'] as const) {
            const options = aiComputeOptions(aiComputeReport({state}))
            expect(options.every((o) => !o.disabled)).toBe(true)
        }
        expect(aiComputeOptions(null)).toHaveLength(5)
    })

    /** Community, or Pro without a licence: no benchmark is coming, so none is promised. */
    it('says it is checking only while the server really is', () => {
        for (const state of ['checking', 'benchmarking'] as const) {
            expect(aiComputeOptions(aiComputeReport({state}))[0].label).toBe('Auto (checking hardware…)')
        }
        expect(aiComputeOptions(aiComputeReport({state: 'unavailable'}))[0].label).toBe('Auto')
        expect(aiComputeOptions(null)[0].label).toBe('Auto')
    })
})

describe('aiComputeSummary', () => {
    it('says where the network runs and how fast', () => {
        expect(aiComputeSummary(laptop())).toBe(
            'Runs on the Integrated GPU: Intel(R) Graphics (ADL GT2) (Vulkan · Mesa 23.2.1, f32), 65 ms per frame.'
        )
    })

    it('puts the server’s notice first, e.g. a forced rung this computer lacks', () => {
        const notice = 'NPU is not available on this computer; using Auto (Integrated GPU).'
        expect(aiComputeSummary(laptop({notice}))).toBe(notice)
    })

    it('says nothing until the benchmark is ready', () => {
        expect(aiComputeSummary(aiComputeReport({state: 'benchmarking'}))).toBe('')
        expect(aiComputeSummary(null)).toBe('')
    })
})

describe('unusableRungs', () => {
    it('names each greyed-out rung, its device and why, and flags what can be installed', () => {
        const rungs = unusableRungs(laptop())
        expect(rungs.map((r) => r.label)).toEqual(['NPU', 'Dedicated GPU'])
        expect(rungs[1]).toMatchObject({device: 'NVIDIA [10de:25ba]', installable: true})
        expect(rungs[0].installable).toBe(false)
        expect(SYSTEM_DEPENDENCIES_URL).toBe('/night-amplifier/system-dependencies')
    })
})
