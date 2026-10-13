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
 * `/ws/events`: the socket, and the one path its messages take. Each message goes, in
 * subscription order, to every handler `on` registered for its type: first the tables of
 * the domains this stream holds (capture, Push-To, installs), then the stores that
 * subscribe themselves (`useAppState`, `useAiCompute`).
 *
 * Handlers run synchronously, one message at a time, so two events arriving in one tick
 * are both seen; the `lastEvent` watcher this replaced saw only the second. The server
 * resends its snapshots after a lagged client, so a handler must be idempotent.
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

    function dispatch(event) {
        for (const handler of subscribers.get(event?.type) ?? []) {
            try {
                handler(event)
            } catch (e) {
                console.error(`Handler for ${event.type} failed:`, e)
            }
        }
    }

    const capture = createCaptureEvents()
    const pushTo = createPushToEvents()
    const installs = createInstallEvents()
    for (const domain of [capture, pushTo, installs]) onEvents({on}, domain.handlers)

    const {connected, error, connect, disconnect} = useWebSocket(WS.EVENTS, {
        onOpen: pushTo.loadStatus,
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
        ...capture.state,
        ...pushTo.state,
        ...installs.state,
        connect,
        disconnect,
    }
}
