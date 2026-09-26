import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'

vi.mock('../api.js', () => ({
    getAiCompute: vi.fn(),
}))

import {getAiCompute} from '../api.js'
import {AI_COMPUTE_POLL_MS, resetAiCompute, useAiCompute} from '../useAiCompute.js'

const report = (state, extra = {}) => ({state, generation: 1, rungs: [], ...extra})

describe('useAiCompute', () => {
    beforeEach(() => {
        vi.useFakeTimers()
        resetAiCompute()
        getAiCompute.mockReset()
    })

    afterEach(() => {
        resetAiCompute()
        vi.useRealTimers()
    })

    it('polls while the server is checking or benchmarking, and stops when ready', async () => {
        getAiCompute
            .mockResolvedValueOnce(report('checking'))
            .mockResolvedValueOnce(report('benchmarking', {progress: {done: 1, total: 3, testing: 'GPU'}}))
            .mockResolvedValueOnce(report('ready'))
        const {refresh, benchmarking, progress, report: current} = useAiCompute()

        await refresh()
        expect(current.value.state).toBe('checking')
        await vi.advanceTimersByTimeAsync(AI_COMPUTE_POLL_MS)
        expect(benchmarking.value).toBe(true)
        expect(progress.value.testing).toBe('GPU')
        await vi.advanceTimersByTimeAsync(AI_COMPUTE_POLL_MS)
        expect(benchmarking.value).toBe(false)
        await vi.advanceTimersByTimeAsync(10 * AI_COMPUTE_POLL_MS)
        expect(getAiCompute).toHaveBeenCalledTimes(3)
    })

    it('shares one report and one request between every user', async () => {
        getAiCompute.mockResolvedValue(report('ready'))
        const first = useAiCompute()
        const second = useAiCompute()
        await Promise.all([first.refresh(), second.refresh()])
        expect(getAiCompute).toHaveBeenCalledTimes(1)
        expect(second.report.value).toBe(first.report.value)
    })

    it('refetches when the server announces a change or the settings move', async () => {
        getAiCompute.mockResolvedValue(report('ready'))
        const {handleEvent} = useAiCompute()
        handleEvent({type: 'ai_compute_changed'})
        await vi.runOnlyPendingTimersAsync()
        handleEvent({type: 'settings_updated'})
        await vi.runOnlyPendingTimersAsync()
        handleEvent({type: 'frame_captured'})
        await vi.runOnlyPendingTimersAsync()
        expect(getAiCompute).toHaveBeenCalledTimes(2)
    })

    /** An older or unreachable server must not leave the UI blocked or polling forever. */
    it('keeps the last report when the request fails, and does not poll a server that is not measuring', async () => {
        getAiCompute.mockRejectedValue(new Error('Server unavailable'))
        const {refresh, report: current, benchmarking} = useAiCompute()
        await refresh()
        expect(current.value).toBe(null)
        expect(benchmarking.value).toBe(false)
        await vi.advanceTimersByTimeAsync(10 * AI_COMPUTE_POLL_MS)
        expect(getAiCompute).toHaveBeenCalledTimes(1)
    })

    it('keeps polling through a failed request mid-benchmark', async () => {
        getAiCompute
            .mockResolvedValueOnce(report('benchmarking'))
            .mockRejectedValueOnce(new Error('blip'))
            .mockResolvedValueOnce(report('ready'))
        const {refresh, benchmarking} = useAiCompute()
        await refresh()
        await vi.advanceTimersByTimeAsync(AI_COMPUTE_POLL_MS)
        expect(benchmarking.value).toBe(true)
        await vi.advanceTimersByTimeAsync(AI_COMPUTE_POLL_MS)
        expect(benchmarking.value).toBe(false)
    })
})
