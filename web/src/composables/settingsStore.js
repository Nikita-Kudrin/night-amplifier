import {readonly, ref} from 'vue'
import {getSettings, updateSettings} from './api.js'

/**
 * The server's settings, with the one write path (`apply`) and the one read path
 * (`refresh`) that keep them in order.
 *
 * - `apply(patch)` posts and adopts the answer, which *is* the settings: an edit used to
 *   cost a POST and two or three GETs (the caller's refresh, then the broadcast's).
 * - `refresh()` never runs beside a write, whose answer it could predate, and never
 *   twice at once: asked again while a GET (or a write) is in flight, it runs once more
 *   after it, and every caller waits for that one.
 * - Every request takes a sequence number when it is sent, and an answer older than the
 *   one adopted is dropped: an older GET resolving last overwrote newer settings.
 *
 * Two writes in flight at once are ordered by when they were sent, which the server may
 * not share; the `settings_updated` each one broadcasts refreshes the settings after both.
 */
export function createSettingsStore() {
    const settings = ref(null)
    // The simulator switch is the panel's to set (before a camera exists to save it on),
    // so it follows the server's value only when one arrives.
    const simulatorEnabled = ref(false)

    let sent = 0
    let adopted = 0
    let writes = 0
    let fetching = null
    let queued = null

    function adopt(at, value) {
        if (!value || at < adopted) return
        adopted = at
        settings.value = value
        if (value.use_simulated_camera !== undefined) {
            simulatorEnabled.value = value.use_simulated_camera
        }
    }

    async function apply(patch) {
        const at = ++sent
        writes++
        try {
            const answer = await updateSettings(patch)
            adopt(at, answer)
            return answer
        } finally {
            writes--
            runQueued()
        }
    }

    function fetchNow() {
        const at = ++sent
        fetching = getSettings()
            .then((value) => {
                adopt(at, value)
                return settings.value
            }, (e) => {
                console.error('Failed to load settings:', e)
                throw e
            })
            .finally(() => {
                fetching = null
                runQueued()
            })
        return fetching
    }

    function runQueued() {
        if (!queued || fetching || writes) return
        const {resolve, reject} = queued
        queued = null
        fetchNow().then(resolve, reject)
    }

    /** Reads the settings again; resolves to them, rejects (logged) when the read failed. */
    function refresh() {
        if (!fetching && !writes) return fetchNow()
        if (!queued) {
            queued = {}
            queued.promise = new Promise((resolve, reject) => Object.assign(queued, {resolve, reject}))
        }
        return queued.promise
    }

    function setSimulatorEnabled(enabled) {
        simulatorEnabled.value = enabled
    }

    return {
        settings: readonly(settings),
        simulatorEnabled,
        apply,
        refresh,
        setSimulatorEnabled,
        handlers: {
            // Another client's edit, a camera connect's seeded profiles, or this page's own
            // write landing: one refresh covers them all, coalesced with any in flight.
            settings_updated: () => refresh().catch(() => {}),
        },
    }
}
