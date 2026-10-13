import {computed, ref, watch} from 'vue'
import {getAiCompute} from './api.js'

/** How often the report is refetched while it is changing and `/ws/events` is down. */
export const AI_COMPUTE_POLL_MS = 1000

function changing(report) {
    return report?.state === 'checking' || report?.state === 'benchmarking'
}

/**
 * The AI compute report, shared by the overlay, the settings panel and App: one report,
 * one request at a time. The server announces every change of it (`ai_compute_changed`,
 * benchmark progress included), so it is polled only while that socket is down; it also
 * depends on the saved compute choice, which no event names, so a changed choice refetches.
 */
export function createAiComputeStore() {
    const report = ref(null)
    let timer = null
    let pending = null
    let events = null

    function schedule() {
        clearTimeout(timer)
        timer = null
        if (changing(report.value) && !events?.connected.value) {
            timer = setTimeout(refresh, AI_COMPUTE_POLL_MS)
        }
    }

    /**
     * Fetch the report now. Concurrent calls share one request. A server without the
     * endpoint (or unreachable) leaves the last report as it was; the overlay then shows
     * nothing rather than blocking on a request that will not succeed.
     */
    function refresh() {
        if (pending) return pending
        pending = getAiCompute()
            .then((value) => {
                report.value = value
            })
            .catch(() => {})
            .finally(() => {
                pending = null
                schedule()
            })
        return pending
    }

    /**
     * Follows `stream` (`useEventStream()`) and the settings' compute choice; the caller's
     * scope owns the subscriptions. A reopened socket refetches what changed while it was down.
     */
    function connectEvents({stream, settings}) {
        events = stream
        stream.on('ai_compute_changed', refresh)
        let opened = stream.connected.value
        watch(stream.connected, (up) => {
            if (up && opened) refresh()
            opened ||= up
            schedule()
        })
        watch(() => settings.value?.denoise?.ai_compute, (choice, before) => {
            if (before !== undefined && choice !== before) refresh()
        })
    }

    function dispose() {
        clearTimeout(timer)
        timer = null
    }

    return {
        report,
        refresh,
        connectEvents,
        dispose,
        benchmarking: computed(() => report.value?.state === 'benchmarking'),
        progress: computed(() => report.value?.progress ?? null),
    }
}

let store = null

export function useAiCompute() {
    store ??= createAiComputeStore()
    return store
}

/** For tests: forget the shared report and stop polling. */
export function __resetForTests() {
    store?.dispose()
    store = null
}
