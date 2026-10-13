import {isRef, ref} from 'vue'
import {vi} from 'vitest'
import {SERVER_EVENTS} from '../composables/api.events.js'
import type {EventOf, ServerEvent, ServerEventType, SyncedEvent} from '../composables/api.events.js'

/**
 * `{type, ...fields}`, typed by `api.events.ts`: a field its event does not carry, or a value
 * of the wrong type, does not compile. Fields may be left out: a handler under test reads
 * only some. A JS caller is not type-checked, so it still meets a runtime refusal of a type
 * the server never sends.
 */
export function serverEvent<K extends ServerEventType>(
    type: K,
    fields: Partial<Omit<EventOf<K>, 'type'>> = {},
): EventOf<K> {
    if (!(type in SERVER_EVENTS)) throw new Error(`the server sends no '${type}' event`)
    return {...fields, type} as EventOf<K>
}

const NOT_SOLVING = {inProgress: false, targetName: null, lastResult: null, stage: null}

type Handler = (event: ServerEvent) => void
type Sync = Pick<SyncedEvent, 'cause' | 'dropped'> & {first: boolean}
type EventStreamStub = ReturnType<typeof stub>

/** Each stub's `on` subscribers by event type, for `emitEvent`. */
const subscribers = new WeakMap<EventStreamStub, Map<ServerEventType, Set<Handler>>>()
/** How many `synced` events each stub has delivered, for `onSync`'s `first`. */
const syncCounts = new WeakMap<EventStreamStub, number>()

/** Its fields are as loosely typed as the JS stream they stand in for, until that is TypeScript. */
function stub(handlers: Map<ServerEventType, Set<Handler>>) {
    const nullable = () => ref<unknown>(null)

    function on<K extends ServerEventType>(type: K, handler: (event: EventOf<K>) => void): () => void {
        let set = handlers.get(type)
        if (!set) handlers.set(type, (set = new Set()))
        // Each set holds one type's handlers, which only `emitEvent` of that type reaches.
        const added = handler as Handler
        set.add(added)
        return () => set.delete(added)
    }

    // Actions read through `stream`, so they reach a ref an override put in place.
    const stream = {
        connected: ref(true),
        error: nullable(),
        captureState: ref('Idle'),
        frameCount: ref(0),
        stackedCount: ref(0),
        rejectedCount: ref(0),
        lastRejectionReason: nullable(),
        droppedCount: ref(0),
        deliveredCount: ref(0),
        lastError: nullable(),
        diskWriterWarning: nullable(),
        unresponsiveWarning: nullable(),
        resumeNotice: nullable(),
        focusModeNotice: nullable(),
        viewedCamera: ref('main'),
        stackCrop: ref<unknown>({crop: null, frame: null}),
        viewedCameraRevision: ref(0),
        pushDirection: nullable(),
        currentTarget: nullable(),
        plateSolving: ref<unknown>({...NOT_SOLVING}),
        pushToBlocked: nullable(),
        solvingMessage: nullable(),
        astapInstallProgress: nullable(),
        catalogInstallProgress: nullable(),
        on: vi.fn(on),
        onSync: vi.fn((fn: (sync: Sync) => void) =>
            on('synced', ({cause, dropped}) => fn({cause, dropped, first: syncCounts.get(stream) === 1}))),
        subscribedTypes: () => [...handlers].filter(([, set]) => set.size > 0).map(([type]) => type),
        connect: vi.fn(),
        disconnect: vi.fn(),
        clearError: vi.fn(() => { stream.lastError.value = null }),
        clearDiskWriterWarning: vi.fn(() => { stream.diskWriterWarning.value = null }),
        clearUnresponsiveWarning: vi.fn(() => { stream.unresponsiveWarning.value = null }),
        clearResumeNotice: vi.fn(() => { stream.resumeNotice.value = null }),
        clearFocusModeNotice: vi.fn(() => { stream.focusModeNotice.value = null }),
        clearPushDirection: vi.fn(() => { stream.pushDirection.value = null }),
        clearAstapInstallProgress: vi.fn(() => { stream.astapInstallProgress.value = null }),
        clearCatalogInstallProgress: vi.fn(() => { stream.catalogInstallProgress.value = null }),
        clearPlateSolving: vi.fn(() => {
            stream.plateSolving.value = {...NOT_SOLVING}
            stream.pushToBlocked.value = null
        }),
        aimAt: vi.fn((target: unknown) => {
            stream.currentTarget.value = target
            stream.pushDirection.value = null
            return target
        }),
        forgetTarget: vi.fn(() => {
            stream.currentTarget.value = null
            stream.pushDirection.value = null
            stream.clearPlateSolving()
        }),
    }
    return stream
}

/**
 * What `useEventStream()` returns, with every field a ref at its initial value and every
 * action a `vi.fn()`: the `clear*` ones empty their field, `aimAt`/`forgetTarget` move
 * the target, `on` and `onSync` subscribe for `emitEvent` and `emitSync`.
 * `eventStream.test.ts` holds the field list to the real composable's, so a field it gains
 * reaches every test.
 *
 * An override is wrapped in a ref unless it already is one (or is a function).
 */
export function createEventStreamStub(overrides: {[K in keyof EventStreamStub]?: unknown} = {}): EventStreamStub {
    const handlers = new Map<ServerEventType, Set<Handler>>()
    const stream = stub(handlers)
    for (const [key, value] of Object.entries(overrides)) {
        Object.assign(stream, {[key]: isRef(value) || typeof value === 'function' ? value : ref(value)})
    }
    subscribers.set(stream, handlers)
    return stream
}

/** Delivers `event` (held to `serverEvent`) to what subscribed to the stub with `on`. */
export function emitEvent<K extends ServerEventType>(
    stream: EventStreamStub,
    event: {type: K} & Partial<Omit<EventOf<K>, 'type'>>,
): void {
    const built: ServerEvent = serverEvent(event.type, event)
    if (built.type === 'synced') syncCounts.set(stream, (syncCounts.get(stream) ?? 0) + 1)
    for (const handler of subscribers.get(stream)?.get(built.type) ?? []) handler(built)
}

/** Ends a sync on the stub, as the server's `synced` does: on connect, or after a lag. */
export function emitSync(stream: EventStreamStub, {cause = 'connect', dropped = 0}: Partial<SyncedEvent> = {}): void {
    emitEvent(stream, {type: 'synced', cause, dropped})
}
