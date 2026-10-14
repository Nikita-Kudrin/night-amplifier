import type {EventOf, ServerEvent, ServerEventType} from '../api.events'

/**
 * A `{type: handler}` table, each handler taking its own event: a field the event does not
 * carry, or a type the server never sends, does not compile. Every domain and store table
 * is one (`… satisfies EventHandlers`).
 */
export type EventHandlers = {[K in ServerEventType]?: (event: EventOf<K>) => void}

type AnyHandler = (event: ServerEvent) => void

/**
 * `table`'s handler for `type`, as one taking any event. The cast is TypeScript's
 * correlated-union limitation (#30581): it cannot see that `table[type]` takes
 * `EventOf<typeof type>`, which `EventHandlers` guarantees.
 */
export function handlerOf(table: EventHandlers, type: ServerEventType): AnyHandler | undefined {
    return table[type] as AnyHandler | undefined
}

/** Every handler in `table`, by its type; see `handlerOf`. */
export function handlerEntries(table: EventHandlers): [ServerEventType, AnyHandler][] {
    const entries: [ServerEventType, AnyHandler][] = []
    for (const type of Object.keys(table) as ServerEventType[]) {
        const handler = handlerOf(table, type)
        if (handler) entries.push([type, handler])
    }
    return entries
}
