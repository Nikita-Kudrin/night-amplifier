import {tryOnScopeDispose} from './lifecycle.js'
import {WS} from '../constants/endpoints.js'
import {useWebSocket} from './useWebSocket.js'
import {createCaptureEvents} from './events/captureEvents.js'
import {createPushToEvents} from './events/pushToEvents.js'
import {createInstallEvents} from './events/installEvents.js'

export {pushDirectionFrom} from './events/pushToEvents.js'

/**
 * Subscribes every handler of a `{type: handler}` table to `stream`.
 * @returns {() => void} Unsubscribes them all.
 */
export function onEvents(stream, table) {
    const offs = Object.entries(table).map(([type, handler]) => stream.on(type, handler))
    return () => offs.forEach((off) => off())
}

/**
 * Calls `refresh(sync)` on every sync of `stream` (see `onSync`), the first only unless
 * `skipFirst()`: that one lands beside the page's own first load, which is reading the
 * same thing already. For state whose events only announce a change (`settings_updated`):
 * one lost while the socket was down, or to a lag, is caught up here.
 * @returns {() => void} Unsubscribes.
 */
export function resyncOn(stream, refresh, {skipFirst = () => false} = {}) {
    return stream.onSync((sync) => {
        if (sync.first && skipFirst()) return
        refresh(sync)
    })
}

/**
 * `/ws/events`: the socket, and the one path its messages take. Each message goes, in
 * subscription order, to every handler `on` registered for its type: first the tables of
 * the domains this stream holds (capture, Push-To, installs), then the stores that
 * subscribe themselves (`useAppState`, `useAiCompute`).
 *
 * Handlers run synchronously, one message at a time, so two events arriving in one tick
 * are both seen; the `lastEvent` watcher this replaced saw only the second. The server
 * resyncs a client on connect and after it lagged (its snapshot, then `synced`; see
 * `onSync`), so a handler must be idempotent.
 */
export function useEventStream() {
    const subscribers = new Map()

    /**
     * Calls `handler(event)` for every event of `type`; inside a component or effect
     * scope, until it goes away.
     * @returns {() => void} Unsubscribes early.
     */
    function on(type, handler) {
        if (!subscribers.has(type)) subscribers.set(type, new Set())
        subscribers.get(type).add(handler)
        const off = () => subscribers.get(type)?.delete(handler)
        tryOnScopeDispose(off)
        return off
    }

    /**
     * The types some handler follows now. For the guard tests that hold the handlers to the
     * server's catalogue (`useEventStream.catalogue.test.js`), not for routing.
     * @returns {string[]}
     */
    function subscribedTypes() {
        return [...subscribers].filter(([, handlers]) => handlers.size > 0).map(([type]) => type)
    }

    function dispatch(event) {
        for (const handler of subscribers.get(event?.type) ?? []) {
            try {
                handler(event)
            } catch (e) {
                console.error(`Handler for ${event.type} failed:`, e)
            }
        }
    }

    let syncs = 0
    on('synced', ({cause, dropped}) => {
        syncs++
        if (cause === 'lagged') console.debug(`Events stream lagged by ${dropped} events; resynced`)
    })

    /**
     * Calls `fn({cause, dropped, first})` on every `synced`, which ends the snapshot the
     * server sends on connect (`cause` 'connect') and after this client fell behind
     * ('lagged'): the moment to re-read what no event restates (`resyncOn`). `first` marks
     * the first sync this stream delivered. Scoped like `on`.
     * @returns {() => void} Unsubscribes early.
     */
    function onSync(fn) {
        return on('synced', ({cause, dropped}) => fn({cause, dropped, first: syncs === 1}))
    }

    const capture = createCaptureEvents()
    const pushTo = createPushToEvents()
    const installs = createInstallEvents()
    for (const domain of [capture, pushTo, installs]) onEvents({on}, domain.handlers)

    const {connected, error, connect, disconnect} = useWebSocket(WS.EVENTS, {
        onMessage: (message) => {
            let event
            try {
                event = JSON.parse(message.data)
            } catch (e) {
                console.error('Failed to parse event:', e)
                return
            }
            dispatch(event)
        },
    })

    return {
        connected,
        error,
        on,
        onSync,
        subscribedTypes,
        ...capture.state,
        ...pushTo.state,
        ...installs.state,
        connect,
        disconnect,
    }
}
