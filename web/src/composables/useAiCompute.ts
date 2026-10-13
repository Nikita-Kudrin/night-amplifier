import {computed, ref, watch} from 'vue'
import type {DeepReadonly, Ref} from 'vue'
import {getAiCompute} from './api.js'
import {onEvents, resyncOn} from './useEventStream.js'
import type {EventStream} from './useEventStream.js'
import type {AiComputeReport, Settings} from './api.types.js'
import type {EventHandlers} from './events/handlers.js'

/** How often the report is refetched while it is changing and `/ws/events` is down. */
export const AI_COMPUTE_POLL_MS = 1000

function changing(report: AiComputeReport | null) {
    return report?.state === 'checking' || report?.state === 'benchmarking'
}

/**
 * The AI compute report, shared by the overlay, the settings panel and App: one report,
 * one request at a time. The server announces every change of it (`ai_compute_changed`,
 * benchmark progress included), so it is polled only while that socket is down; it also
 * depends on the saved compute choice, which no event names, so a changed choice refetches.
 */
export function createAiComputeStore() {
    const report = ref<AiComputeReport | null>(null)
    let timer: ReturnType<typeof setTimeout> | undefined
    let inFlight: Promise<void> | null = null
    let stale = false
    let events: EventStream | null = null

    function schedule() {
        clearTimeout(timer)
        timer = undefined
        if (changing(report.value) && !events?.connected.value) {
            timer = setTimeout(refresh, AI_COMPUTE_POLL_MS)
        }
    }

    async function drain() {
        while (stale) {
            stale = false
            try {
                report.value = await getAiCompute()
            } catch {
                // Keeps the last report; see `refresh`.
            }
        }
        inFlight = null
        schedule()
    }

    /**
     * Fetch the report now; the promise settles once it is in place. One request at a
     * time: a call during one reads again after it, since that answer may predate what
     * the caller learnt (an `ai_compute_changed` racing the read once left the report
     * `benchmarking` behind the overlay, with nothing polling). A server without the
     * endpoint (or unreachable) leaves the last report as it was; the overlay then shows
     * nothing rather than blocking on a request that will not succeed.
     */
    function refresh(): Promise<void> {
        stale = true
        inFlight ??= drain()
        return inFlight
    }

    /** The first sync retries a first load that failed, once whatever is in flight settles. */
    async function loadIfMissing() {
        await inFlight
        if (report.value === null) await refresh()
    }

    const handlers = {
        ai_compute_changed: refresh,
    } satisfies EventHandlers

    /**
     * Follows `stream` (`useEventStream()`) and the settings' compute choice; the caller's
     * scope owns the subscriptions. A sync refetches what changed while the socket was
     * down or this client lagged.
     */
    function connectEvents({stream, settings}: {
        stream: EventStream
        settings: Readonly<Ref<DeepReadonly<Settings> | null>>
    }) {
        events = stream
        onEvents(stream, handlers)
        resyncOn(stream, (sync) => (sync.first ? loadIfMissing() : refresh()))
        watch(stream.connected, schedule)
        watch(() => settings.value?.denoise?.ai_compute, (choice, before) => {
            if (before !== undefined && choice !== before) refresh()
        })
    }

    function dispose() {
        clearTimeout(timer)
        timer = undefined
    }

    return {
        report,
        refresh,
        handlers,
        connectEvents,
        dispose,
        benchmarking: computed(() => report.value?.state === 'benchmarking'),
        progress: computed(() => report.value?.progress ?? null),
    }
}

let store: ReturnType<typeof createAiComputeStore> | null = null

export function useAiCompute() {
    store ??= createAiComputeStore()
    return store
}

/** For tests: forget the shared report and stop polling. */
export function __resetForTests() {
    store?.dispose()
    store = null
}
