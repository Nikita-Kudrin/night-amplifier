import {computed, isRef, ref} from 'vue'
import type {UnwrapRef} from 'vue'
import {vi} from 'vitest'
import type {Mock} from 'vitest'
import type {EventOf, ServerEvent, ServerEventType, SyncedEvent} from '../composables/api.events'
import type {CameraRole, CaptureState} from '../composables/api.types'
import type {EventHandlers, EventStream, Sync} from '../composables/useEventStream'
import {handlerOf} from '../composables/events/handlers'
import type {EventHandler} from '../composables/events/handlers'
import {SERVER_EVENT_EXAMPLES} from './api.examples'
import type {StackCrop, StorageSync} from '../composables/events/captureEvents'
import {NOT_SOLVING} from '../composables/events/pushToEvents'
import type {ClientPushDirection, CurrentTarget, PlateSolving} from '../composables/events/pushToEvents'
import type {AstapInstallProgress, CatalogInstallProgress} from '../composables/events/installEvents'

/**
 * The fields of `K`'s event a test may give, any of them: a type the server never sends, a
 * field its event does not carry, or a value of the wrong type, does not compile. A union
 * event is typed by the branch the fields name (`state: 'failed'` with its `error`).
 */
export type Fields<K extends ServerEventType> = EventOf<K> extends infer E
    ? E extends unknown ? Partial<Omit<E, 'type'>> : never
    : never

/** An event as a test writes it: its `type`, and the fields `serverEvent` takes. */
export type EventFields<K extends ServerEventType> = {type: K} & Fields<K>

/**
 * `K`'s event as the server sends it: `fields` over the generated example's required fields
 * (`SERVER_EVENT_EXAMPLES`, a nullable one `null`), so a handler never meets a field the
 * wire always carries left out. A union event's example is one branch: naming another
 * (`state: 'running'`) means naming what that branch carries too.
 */
export function serverEvent<K extends ServerEventType>(type: K, fields?: Fields<K>): EventOf<K> {
    // A spread over a generic event is not assignable to it; both halves are typed above.
    return {...structuredClone(SERVER_EVENT_EXAMPLES[type]), ...fields, type} as EventOf<K>
}

type Handler = EventHandler<ServerEvent>

/** `vi.fn(fn)`, typed as `fn` too: `Mock` keeps only one signature, which drops `fn`'s generics. */
function spy<F extends (...args: never[]) => unknown>(fn: F): Mock<F> & F {
    return vi.fn(fn) as Mock<F> & F
}
type Overrides = {[K in keyof EventStream]?: EventStream[K] | UnwrapRef<EventStream[K]>}

/** A stub's `on` subscribers by event type, and how many `synced` it delivered (`onSync`'s `first`). */
interface Wiring {
    handlers: Map<ServerEventType, Set<Handler>>
    syncs: number
}

/** Each stub's wiring, for `emitEvent`. */
const wirings = new WeakMap<EventStream, Wiring>()

function stub(wiring: Wiring): EventStream {
    const {handlers} = wiring

    function on<K extends ServerEventType>(type: K, handler: EventHandler<EventOf<K>>): () => void {
        let set = handlers.get(type)
        if (!set) handlers.set(type, (set = new Set()))
        // Each set holds one type's handlers, which only `emitEvent` of that type reaches.
        const added = handler as Handler
        set.add(added)
        return () => set.delete(added)
    }

    // Actions read through `stream`, so they reach a ref an override put in place. Typed
    // `EventStream`, so a field the real stream gains, or loses, is one this must too.
    const stream: EventStream = {
        connected: ref(true),
        error: ref(null),
        captureState: ref<CaptureState>('Idle'),
        frameCount: ref(0),
        stackedCount: ref(0),
        rejectedCount: ref(0),
        lastRejectionReason: ref<string | null>(null),
        droppedCount: ref(0),
        deliveredCount: ref(0),
        lastError: ref<string | null>(null),
        diskWriterWarning: ref<number | null>(null),
        storageSync: ref<Record<CameraRole, StorageSync | null>>({main: null, guide: null}),
        unresponsiveWarning: ref<string | null>(null),
        resumeNotice: ref<string | null>(null),
        focusModeNotice: ref<string | null>(null),
        viewedCamera: ref<CameraRole>('main'),
        stackCrop: ref<StackCrop>({crop: null, frame: null}),
        viewedCameraRevision: ref(0),
        pushDirection: ref<ClientPushDirection | null>(null),
        currentTarget: ref<CurrentTarget | null>(null),
        plateSolving: ref<PlateSolving>({...NOT_SOLVING}),
        pushToBlocked: ref<string | null>(null),
        solvingMessage: computed<string | null>(() => null),
        astapInstallProgress: ref<AstapInstallProgress | null>(null),
        catalogInstallProgress: ref<CatalogInstallProgress | null>(null),
        on: spy(on),
        onSync: vi.fn((fn: (sync: Sync) => void) =>
            on('synced', ({cause, dropped}) => fn({cause, dropped, first: wiring.syncs === 1}))),
        subscribedTypes: () => [...handlers].filter(([, set]) => set.size > 0).map(([type]) => type),
        connect: vi.fn(),
        disconnect: vi.fn(),
        clearError: vi.fn(() => (stream.lastError.value = null)),
        clearDiskWriterWarning: vi.fn(() => (stream.diskWriterWarning.value = null)),
        clearStorageSync: vi.fn((role: CameraRole) => {
            stream.storageSync.value = {...stream.storageSync.value, [role]: null}
        }),
        clearUnresponsiveWarning: vi.fn(() => { stream.unresponsiveWarning.value = null }),
        clearResumeNotice: vi.fn(() => (stream.resumeNotice.value = null)),
        clearFocusModeNotice: vi.fn(() => (stream.focusModeNotice.value = null)),
        clearPushDirection: vi.fn(() => { stream.pushDirection.value = null }),
        clearAstapInstallProgress: vi.fn(() => (stream.astapInstallProgress.value = null)),
        clearCatalogInstallProgress: vi.fn(() => (stream.catalogInstallProgress.value = null)),
        clearPlateSolving: vi.fn(() => {
            stream.plateSolving.value = {...NOT_SOLVING}
            stream.pushToBlocked.value = null
        }),
        aimAt: spy(<T extends CurrentTarget>(target: T): T => {
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
 *
 * An override is wrapped in a ref unless it already is one (or is a function).
 */
export function createEventStreamStub(overrides: Overrides = {}): EventStream {
    const wiring: Wiring = {handlers: new Map(), syncs: 0}
    const stream = stub(wiring)
    for (const [key, value] of Object.entries(overrides)) {
        Object.assign(stream, {[key]: isRef(value) || typeof value === 'function' ? value : ref(value)})
    }
    wirings.set(stream, wiring)
    return stream
}

/** Hands `table`'s handler for `type` the event `serverEvent` builds, as the stream would; throws without one. */
export function sendTo<K extends ServerEventType>(
    table: EventHandlers,
    type: K,
    fields?: Fields<K>,
): void {
    const handler = handlerOf(table, type)
    if (!handler) throw new Error(`no handler for '${type}'`)
    handler(serverEvent(type, fields))
}

/** Delivers `event` (held to `serverEvent`) to what subscribed to the stub with `on`. */
export function emitEvent<K extends ServerEventType>(stream: EventStream, event: EventFields<K>): void {
    const built: ServerEvent = serverEvent(event.type, event)
    const wiring = wirings.get(stream)
    if (!wiring) throw new Error('not a stream createEventStreamStub built')
    if (built.type === 'synced') wiring.syncs++
    for (const handler of wiring.handlers.get(built.type) ?? []) handler(built)
}

/** Ends a sync on the stub, as the server's `synced` does: on connect, or after a lag. */
export function emitSync(stream: EventStream, {cause = 'connect', dropped = 0}: Partial<SyncedEvent> = {}): void {
    emitEvent(stream, {type: 'synced', cause, dropped})
}
