import {tryOnScopeDispose} from './lifecycle'
import {WS} from '../constants/endpoints'
import {useWebSocket} from './useWebSocket'
import {createCaptureEvents} from './events/captureEvents'
import {createPushToEvents} from './events/pushToEvents'
import {createInstallEvents} from './events/installEvents'
import {handlerEntries} from './events/handlers'
import type {EventHandler, EventHandlers} from './events/handlers'
import type {EventOf, ServerEvent, ServerEventType, SyncedEvent} from './api.events'

export {pushDirectionFrom} from './events/pushToEvents'
export type {EventHandlers} from './events/handlers'

/** What `onSync` hands its callback: the `synced` event's fields, and whether it is the stream's first. */
export type Sync = Pick<SyncedEvent, 'cause' | 'dropped'> & {first: boolean}

export type EventStream = ReturnType<typeof useEventStream>

function isEvent(value: unknown): value is ServerEvent {
    return typeof value === 'object' && value !== null && typeof (value as {type?: unknown}).type === 'string'
}

/**
 * Subscribes every handler of a `{type: handler}` table to `stream`.
 * @returns Unsubscribes them all.
 */
export function onEvents(stream: Pick<EventStream, 'on'>, table: EventHandlers): () => void {
    const offs = handlerEntries(table).map(([type, handler]) => stream.on(type, handler))
    return () => offs.forEach((off) => off())
}

/**
 * Calls `refresh(sync)` on every sync of `stream` (see `onSync`), the first only unless
 * `skipFirst()`: that one lands beside the page's own first load, which is reading the
 * same thing already. For state whose events only announce a change (`settings_updated`):
 * one lost while the socket was down, or to a lag, is caught up here.
 * @returns Unsubscribes.
 */
export function resyncOn(
    stream: Pick<EventStream, 'onSync'>,
    refresh: (sync: Sync) => unknown,
    {skipFirst = () => false}: {skipFirst?: () => boolean} = {},
): () => void {
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
    // Each set holds one type's handlers, which `dispatch` hands only events of that type.
    const subscribers = new Map<ServerEventType, Set<EventHandler<ServerEvent>>>()

    /**
     * Calls `handler(event)` for every event of `type`; inside a component or effect
     * scope, until it goes away.
     * @returns Unsubscribes early.
     */
    function on<K extends ServerEventType>(type: K, handler: EventHandler<EventOf<K>>): () => void {
        let handlers = subscribers.get(type)
        if (!handlers) subscribers.set(type, (handlers = new Set()))
        const added = handler as EventHandler<ServerEvent>
        handlers.add(added)
        const off = () => {
            handlers.delete(added)
        }
        tryOnScopeDispose(off)
        return off
    }

    /**
     * The types some handler follows now. For the guard tests that hold the handlers to the
     * server's catalogue (`useEventStream.catalogue.test.ts`), not for routing.
     */
    function subscribedTypes(): ServerEventType[] {
        return [...subscribers].filter(([, handlers]) => handlers.size > 0).map(([type]) => type)
    }

    // A handler that fails, by throwing or by a promise it returns rejecting, is logged and
    // skipped: unhandled, a rejection named no event and failed the test run in its place.
    function dispatch(event: ServerEvent) {
        const failed = (e: unknown) => console.error(`Handler for ${event.type} failed:`, e)
        for (const handler of subscribers.get(event.type) ?? []) {
            try {
                const result: unknown = handler(event)
                if (result instanceof Promise) result.catch(failed)
            } catch (e) {
                failed(e)
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
     * @returns Unsubscribes early.
     */
    function onSync(fn: (sync: Sync) => void): () => void {
        return on('synced', ({cause, dropped}) => fn({cause, dropped, first: syncs === 1}))
    }

    const capture = createCaptureEvents()
    const pushTo = createPushToEvents()
    const installs = createInstallEvents()
    for (const domain of [capture, pushTo, installs]) onEvents({on}, domain.handlers)

    const {connected, error, connect, disconnect} = useWebSocket(WS.EVENTS, {
        onMessage: (message: MessageEvent<string>) => {
            let parsed: unknown
            try {
                parsed = JSON.parse(message.data)
            } catch (e) {
                console.error('Failed to parse event:', e)
                return
            }
            // The server's own events, typed by `api.events.ts`; anything without a `type`
            // (a stray `null`, a number) has no handler to reach.
            if (isEvent(parsed)) dispatch(parsed)
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
