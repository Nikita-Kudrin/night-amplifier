import {ref, computed} from 'vue'
import {getAiCompute} from './api.js'

/** How often the report is refetched while the server is still checking or measuring. */
export const AI_COMPUTE_POLL_MS = 1000

// Shared by the overlay, the settings panel and App: one report, one poll.
const report = ref(null)
let timer = null
let pending = null

function pollingState(value) {
    return value?.state === 'checking' || value?.state === 'benchmarking'
}

function schedule() {
    clearTimeout(timer)
    timer = null
    if (pollingState(report.value)) {
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

/** React to server events: the report changed, or the saved choice did. */
function handleEvent(event) {
    if (event?.type === 'ai_compute_changed' || event?.type === 'settings_updated') {
        refresh()
    }
}

export function useAiCompute() {
    return {
        report,
        refresh,
        handleEvent,
        benchmarking: computed(() => report.value?.state === 'benchmarking'),
        progress: computed(() => report.value?.progress ?? null),
    }
}

/** For tests: forget the shared report and stop polling. */
export function resetAiCompute() {
    clearTimeout(timer)
    timer = null
    pending = null
    report.value = null
}
