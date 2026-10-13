import {isRef, ref} from 'vue'
import {vi} from 'vitest'
import {SERVER_EVENTS} from '../composables/api.events.js'
import {wireEventTypes} from './apiTypes.js'

const eventTypes = wireEventTypes()

/**
 * `{type, ...fields}`, held to `api.events.js`: throws on a type the server never sends or a
 * field its event does not carry, so a test cannot keep passing on a renamed or invented
 * event. Fields may be left out: a handler under test reads only some.
 */
export function serverEvent(type, fields = {}) {
    if (!(type in SERVER_EVENTS)) throw new Error(`the server sends no '${type}' event`)
    const name = `${type.replace(/(^|_)(\w)/g, (_, __, letter) => letter.toUpperCase())}Event`
    const unknown = Object.keys(fields).filter((key) => key !== 'type' && !eventTypes[name].all.includes(key))
    if (unknown.length > 0) throw new Error(`'${type}' carries no ${unknown.join(', ')}`)
    return {type, ...fields}
}

/** The `clear*` actions and the field each one empties. */
const CLEARS = {
    clearError: 'lastError',
    clearDiskWriterWarning: 'diskWriterWarning',
    clearUnresponsiveWarning: 'unresponsiveWarning',
    clearResumeNotice: 'resumeNotice',
    clearFocusModeNotice: 'focusModeNotice',
    clearPushDirection: 'pushDirection',
    clearAstapInstallProgress: 'astapInstallProgress',
    clearCatalogInstallProgress: 'catalogInstallProgress',
}

const NOT_SOLVING = {inProgress: false, targetName: null, lastResult: null, stage: null}

/** Each stub's `on` subscribers by event type, for `emitEvent`. */
const subscribers = new WeakMap()
/** How many `synced` events each stub has delivered, for `onSync`'s `first`. */
const syncCounts = new WeakMap()

/**
 * What `useEventStream()` returns, with every field a ref at its initial value and every
 * action a `vi.fn()`: the `clear*` ones empty their field, `aimAt`/`forgetTarget` move
 * the target, `on` and `onSync` subscribe for `emitEvent` and `emitSync`.
 * `eventStream.test.js` holds the field list to the real composable's, so a field it gains
 * reaches every test.
 *
 * An override is wrapped in a ref unless it already is one (or is a function).
 */
export function createEventStreamStub(overrides = {}) {
    const handlers = new Map()
    const stream = {
        connected: ref(true),
        error: ref(null),
        captureState: ref('Idle'),
        frameCount: ref(0),
        stackedCount: ref(0),
        rejectedCount: ref(0),
        lastRejectionReason: ref(null),
        droppedCount: ref(0),
        deliveredCount: ref(0),
        lastError: ref(null),
        diskWriterWarning: ref(null),
        unresponsiveWarning: ref(null),
        resumeNotice: ref(null),
        focusModeNotice: ref(null),
        viewedCamera: ref('main'),
        stackCrop: ref({crop: null, frame: null}),
        viewedCameraRevision: ref(0),
        pushDirection: ref(null),
        currentTarget: ref(null),
        plateSolving: ref({...NOT_SOLVING}),
        pushToBlocked: ref(null),
        solvingMessage: ref(null),
        astapInstallProgress: ref(null),
        catalogInstallProgress: ref(null),
        on: vi.fn((type, handler) => {
            if (!handlers.has(type)) handlers.set(type, new Set())
            handlers.get(type).add(handler)
            return () => handlers.get(type).delete(handler)
        }),
        onSync: vi.fn((fn) =>
            stream.on('synced', ({cause, dropped}) => fn({cause, dropped, first: syncCounts.get(stream) === 1}))),
        subscribedTypes: () => [...handlers].filter(([, set]) => set.size > 0).map(([type]) => type),
        connect: vi.fn(),
        disconnect: vi.fn(),
    }
    for (const [action, field] of Object.entries(CLEARS)) {
        stream[action] = vi.fn(() => {
            stream[field].value = null
        })
    }
    stream.clearPlateSolving = vi.fn(() => {
        stream.plateSolving.value = {...NOT_SOLVING}
        stream.pushToBlocked.value = null
    })
    stream.aimAt = vi.fn((target) => {
        stream.currentTarget.value = target
        stream.pushDirection.value = null
        return target
    })
    stream.forgetTarget = vi.fn(() => {
        stream.currentTarget.value = null
        stream.pushDirection.value = null
        stream.clearPlateSolving()
    })
    for (const [key, value] of Object.entries(overrides)) {
        stream[key] = isRef(value) || typeof value === 'function' ? value : ref(value)
    }
    subscribers.set(stream, handlers)
    return stream
}

/** Delivers `event` (held to `serverEvent`) to what subscribed to the stub with `on`. */
export function emitEvent(stream, event) {
    serverEvent(event.type, event)
    if (event.type === 'synced') syncCounts.set(stream, (syncCounts.get(stream) ?? 0) + 1)
    for (const handler of subscribers.get(stream)?.get(event.type) ?? []) handler(event)
}

/** Ends a sync on the stub, as the server's `synced` does: on connect, or after a lag. */
export function emitSync(stream, {cause = 'connect', dropped = 0} = {}) {
    emitEvent(stream, {type: 'synced', cause, dropped})
}
