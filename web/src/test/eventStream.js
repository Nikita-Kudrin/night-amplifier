import {isRef, ref} from 'vue'
import {vi} from 'vitest'

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

/**
 * What `useEventStream()` returns, with every field a ref at its initial value and every
 * action a `vi.fn()`: the `clear*` ones empty their field, `aimAt`/`forgetTarget` move
 * the target, `on` subscribes for `emitEvent`. `eventStream.test.js` holds the field list
 * to the real composable's, so a field it gains reaches every test.
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

/** Delivers `event` to what subscribed to the stub with `on`, as the socket would. */
export function emitEvent(stream, event) {
    for (const handler of subscribers.get(stream)?.get(event.type) ?? []) handler(event)
}
