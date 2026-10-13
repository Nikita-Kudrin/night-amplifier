import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'

vi.mock('../api.js', async (importOriginal) =>
    (await import('../../test/mockApi.js')).mockApi(await importOriginal(), {
        getAiCompute: vi.fn(),
    }))

import {effectScope, nextTick, ref} from 'vue'
import {getAiCompute} from '../api.js'
import {AI_COMPUTE_POLL_MS, __resetForTests as resetAiCompute, useAiCompute} from '../useAiCompute.js'
import {createEventStreamStub, emitEvent} from '../../test/eventStream.js'

const report = (state, extra = {}) => ({state, generation: 1, rungs: [], ...extra})

describe('useAiCompute', () => {
    let scope

    beforeEach(() => {
        vi.useFakeTimers()
        resetAiCompute()
        getAiCompute.mockReset()
        scope = effectScope()
    })

    afterEach(() => {
        scope.stop()
        resetAiCompute()
        vi.useRealTimers()
    })

    /** The store following a stub event stream (connected unless told) and these settings. */
    function connected({up = true, settings = ref(null)} = {}) {
        const stream = createEventStreamStub({connected: up})
        const store = useAiCompute()
        scope.run(() => store.connectEvents({stream, settings}))
        return {store, stream, settings}
    }

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

    it('refetches when the server announces a change, and on nothing else', async () => {
        getAiCompute.mockResolvedValue(report('ready'))
        const {stream} = connected()

        emitEvent(stream, {type: 'ai_compute_changed'})
        await vi.runOnlyPendingTimersAsync()
        emitEvent(stream, {type: 'settings_updated'})
        emitEvent(stream, {type: 'frame_captured'})
        await vi.runOnlyPendingTimersAsync()

        expect(getAiCompute).toHaveBeenCalledTimes(1)
    })

    // The report names the effective unit, which follows the saved choice; no event says so.
    it('refetches when the saved compute choice changes, not on every settings change', async () => {
        getAiCompute.mockResolvedValue(report('ready'))
        const {settings} = connected()
        settings.value = {denoise: {ai_compute: 'auto'}, gain: 0}
        await nextTick()
        settings.value = {denoise: {ai_compute: 'auto'}, gain: 100}
        await nextTick()
        expect(getAiCompute).not.toHaveBeenCalled()

        settings.value = {denoise: {ai_compute: 'cpu'}, gain: 100}
        await nextTick()
        expect(getAiCompute).toHaveBeenCalledTimes(1)
    })

    // Every change, progress included, is announced within 500 ms on the socket.
    it('does not poll a benchmark while the event socket is up', async () => {
        getAiCompute.mockResolvedValue(report('benchmarking'))
        const {store} = connected()

        await store.refresh()
        await vi.advanceTimersByTimeAsync(10 * AI_COMPUTE_POLL_MS)

        expect(getAiCompute).toHaveBeenCalledTimes(1)
    })

    it('polls a benchmark while the socket is down, and refetches once it reopens', async () => {
        getAiCompute.mockResolvedValue(report('benchmarking'))
        const {store, stream} = connected()
        await store.refresh()

        stream.connected.value = false
        await nextTick()
        await vi.advanceTimersByTimeAsync(2 * AI_COMPUTE_POLL_MS)
        expect(getAiCompute).toHaveBeenCalledTimes(3)

        getAiCompute.mockResolvedValue(report('ready'))
        stream.connected.value = true
        await nextTick()
        await vi.runOnlyPendingTimersAsync()
        expect(store.report.value.state).toBe('ready')
        await vi.advanceTimersByTimeAsync(10 * AI_COMPUTE_POLL_MS)
        expect(getAiCompute).toHaveBeenCalledTimes(4)
    })

    it('does not refetch when the socket first opens: App loads the report itself', async () => {
        connected({up: false}).stream.connected.value = true
        await nextTick()
        expect(getAiCompute).not.toHaveBeenCalled()
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
