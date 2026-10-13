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

/**
 * What `useEventStream()` returns, with every field a ref at its initial value and every
 * action a `vi.fn()`, the `clear*` ones emptying their field. `eventStream.test.js` holds
 * the field list to the real composable's, so a field it gains reaches every test.
 *
 * An override is wrapped in a ref unless it already is one (or is a function).
 */
export function createEventStreamStub(overrides = {}) {
    const stream = {
        connected: ref(true),
        error: ref(null),
        lastEvent: ref(null),
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
        plateSolving: ref({inProgress: false, targetName: null, lastResult: null, stage: null}),
        pushToBlocked: ref(null),
        solvingMessage: ref(null),
        astapInstallProgress: ref(null),
        catalogInstallProgress: ref(null),
        connect: vi.fn(),
        disconnect: vi.fn(),
    }
    for (const [action, field] of Object.entries(CLEARS)) {
        stream[action] = vi.fn(() => {
            stream[field].value = null
        })
    }
    stream.clearPlateSolving = vi.fn(() => {
        stream.plateSolving.value = {inProgress: false, targetName: null, lastResult: null, stage: null}
        stream.pushToBlocked.value = null
    })
    for (const [key, value] of Object.entries(overrides)) {
        stream[key] = isRef(value) || typeof value === 'function' ? value : ref(value)
    }
    return stream
}
