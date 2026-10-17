import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'

vi.mock('../api', async (importOriginal) =>
    (await import('../../test/mockApi')).mockApi(await importOriginal(), {
        getAiCompute: vi.fn(),
    }))

import {effectScope, nextTick, ref} from 'vue'
import {getAiCompute} from '../api'
import {AI_COMPUTE_POLL_MS, __resetForTests as resetAiCompute, useAiCompute} from '../useAiCompute'
import {createEventStreamStub, emitEvent, emitSync} from '../../test/eventStream'
import type {EventStream} from '../useEventStream'
import {aiComputeReport, settings as wireSettings} from '../../test/fixtures'
import {pending} from '../../test/http'
import type {AiComputeReport, AiComputePreference, BenchmarkState, Settings} from '../api.types'
import type {EffectScope} from 'vue'

const report = (state: BenchmarkState, extra: Partial<AiComputeReport> = {}) =>
    aiComputeReport({state, generation: 1, ...extra})

/** The settings with this compute choice and gain. */
const withChoice = (ai_compute: AiComputePreference, gain: number) =>
    wireSettings({gain, denoise: {...wireSettings().denoise, ai_compute}})

describe('useAiCompute', () => {
    let scope: EffectScope

    beforeEach(() => {
        vi.useFakeTimers()
        resetAiCompute()
        vi.mocked(getAiCompute).mockReset()
        scope = effectScope()
    })

    afterEach(() => {
        scope.stop()
        resetAiCompute()
        vi.useRealTimers()
    })

    /** The store following a stub event stream (connected unless told) and these settings. */
    function connected({up = true, settings = ref<Settings | null>(null)} = {}) {
        const stream = createEventStreamStub({connected: up})
        const store = useAiCompute()
        scope.run(() => store.connectEvents({stream, settings}))
        return {store, stream, settings}
    }

    it('polls while the server is checking or benchmarking, and stops when ready', async () => {
        vi.mocked(getAiCompute)
            .mockResolvedValueOnce(report('checking'))
            .mockResolvedValueOnce(report('benchmarking', {progress: {done: 1, total: 3, testing: 'GPU'}}))
            .mockResolvedValueOnce(report('ready'))
        const {refresh, benchmarking, progress, report: current} = useAiCompute()

        await refresh()
        expect(current.value?.state).toBe('checking')
        await vi.advanceTimersByTimeAsync(AI_COMPUTE_POLL_MS)
        expect(benchmarking.value).toBe(true)
        expect(progress.value?.testing).toBe('GPU')
        await vi.advanceTimersByTimeAsync(AI_COMPUTE_POLL_MS)
        expect(benchmarking.value).toBe(false)
        await vi.advanceTimersByTimeAsync(10 * AI_COMPUTE_POLL_MS)
        expect(getAiCompute).toHaveBeenCalledTimes(3)
    })

    it('shares one report between every user, one request at a time', async () => {
        const firstRead = pending<AiComputeReport>()
        vi.mocked(getAiCompute)
            .mockReturnValueOnce(firstRead.promise)
            .mockResolvedValue(report('ready'))
        const first = useAiCompute()
        const second = useAiCompute()

        const both = Promise.all([first.refresh(), second.refresh(), second.refresh()])
        expect(getAiCompute).toHaveBeenCalledTimes(1)
        firstRead.resolve(report('benchmarking'))
        await both

        // The calls made during the first read share one more, so each sees a newer answer.
        expect(getAiCompute).toHaveBeenCalledTimes(2)
        expect(second.report.value).toBe(first.report.value)
        expect(first.report.value?.state).toBe('ready')
    })

    // The benchmark finished after the server read the report: the older read lands last,
    // and with the socket up nothing polls, so App stayed `inert` behind the overlay.
    it('reads the report again when a change is announced during a request', async () => {
        const {store, stream} = connected()
        const firstRead = pending<AiComputeReport>()
        vi.mocked(getAiCompute)
            .mockReturnValueOnce(firstRead.promise)
            .mockResolvedValue(report('ready'))

        const first = store.refresh()
        emitEvent(stream, {type: 'ai_compute_changed'})
        firstRead.resolve(report('benchmarking'))
        await first

        expect(store.benchmarking.value).toBe(false)
        expect(getAiCompute).toHaveBeenCalledTimes(2)
    })

    it('refetches when the server announces a change, and on nothing else', async () => {
        vi.mocked(getAiCompute).mockResolvedValue(report('ready'))
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
        vi.mocked(getAiCompute).mockResolvedValue(report('ready'))
        const {settings} = connected()
        settings.value = withChoice('auto', 0)
        await nextTick()
        settings.value = withChoice('auto', 100)
        await nextTick()
        expect(getAiCompute).not.toHaveBeenCalled()

        settings.value = withChoice('cpu', 100)
        await nextTick()
        expect(getAiCompute).toHaveBeenCalledTimes(1)
    })

    // Every change, progress included, is announced within 500 ms on the socket.
    it('does not poll a benchmark while the event socket is up', async () => {
        vi.mocked(getAiCompute).mockResolvedValue(report('benchmarking'))
        const {store} = connected()

        await store.refresh()
        await vi.advanceTimersByTimeAsync(10 * AI_COMPUTE_POLL_MS)

        expect(getAiCompute).toHaveBeenCalledTimes(1)
    })

    /** The socket coming up, and the server's sync that follows. */
    async function reopen(stream: EventStream) {
        stream.connected.value = true
        await nextTick()
        emitSync(stream)
    }

    it('polls a benchmark while the socket is down, and refetches on the sync once it reopens', async () => {
        vi.mocked(getAiCompute).mockResolvedValue(report('benchmarking'))
        const {store, stream} = connected()
        await store.refresh()
        emitSync(stream)

        stream.connected.value = false
        await nextTick()
        await vi.advanceTimersByTimeAsync(2 * AI_COMPUTE_POLL_MS)
        expect(getAiCompute).toHaveBeenCalledTimes(3)

        vi.mocked(getAiCompute).mockResolvedValue(report('ready'))
        await reopen(stream)
        await vi.runOnlyPendingTimersAsync()
        expect(store.report.value?.state).toBe('ready')
        await vi.advanceTimersByTimeAsync(10 * AI_COMPUTE_POLL_MS)
        expect(getAiCompute).toHaveBeenCalledTimes(4)
    })

    // The socket stayed up, but the `ai_compute_changed` that ended the benchmark was lost.
    it('refetches on a sync after this client lagged', async () => {
        vi.mocked(getAiCompute).mockResolvedValueOnce(report('benchmarking')).mockResolvedValue(report('ready'))
        const {store, stream} = connected()
        await store.refresh()
        emitSync(stream)
        await vi.runOnlyPendingTimersAsync()

        emitSync(stream, {cause: 'lagged', dropped: 144})
        await vi.runOnlyPendingTimersAsync()

        expect(getAiCompute).toHaveBeenCalledTimes(2)
        expect(store.benchmarking.value).toBe(false)
    })

    it('does not refetch a report App loaded on the first sync', async () => {
        vi.mocked(getAiCompute).mockResolvedValue(report('ready'))
        const {store, stream} = connected({up: false})
        await store.refresh()

        await reopen(stream)
        await vi.runOnlyPendingTimersAsync()

        expect(getAiCompute).toHaveBeenCalledTimes(1)
    })

    it('waits out a first load in flight on the first sync, without a second read', async () => {
        const read = pending<AiComputeReport>()
        vi.mocked(getAiCompute).mockReturnValueOnce(read.promise)
        const {store, stream} = connected({up: false})
        const loading = store.refresh()

        await reopen(stream)
        read.resolve(report('ready'))
        await loading
        await vi.runOnlyPendingTimersAsync()

        expect(getAiCompute).toHaveBeenCalledTimes(1)
    })

    // With the socket up nothing polls, so a failed first load was never retried.
    it('retries a failed first load on the first sync', async () => {
        vi.mocked(getAiCompute).mockRejectedValueOnce(new Error('Server starting')).mockResolvedValue(report('ready'))
        const {store, stream} = connected({up: false})
        const loading = store.refresh()

        await reopen(stream)
        await loading
        await vi.runOnlyPendingTimersAsync()

        expect(getAiCompute).toHaveBeenCalledTimes(2)
        expect(store.report.value?.state).toBe('ready')
    })

    /** An older or unreachable server must not leave the UI blocked or polling forever. */
    it('keeps the last report when the request fails, and does not poll a server that is not measuring', async () => {
        vi.mocked(getAiCompute).mockRejectedValue(new Error('Server unavailable'))
        const {refresh, report: current, benchmarking} = useAiCompute()
        await refresh()
        expect(current.value).toBe(null)
        expect(benchmarking.value).toBe(false)
        await vi.advanceTimersByTimeAsync(10 * AI_COMPUTE_POLL_MS)
        expect(getAiCompute).toHaveBeenCalledTimes(1)
    })

    it('keeps polling through a failed request mid-benchmark', async () => {
        vi.mocked(getAiCompute)
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
