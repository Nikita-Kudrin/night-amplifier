import {describe, it, expect, vi, beforeEach} from 'vitest'
import {createCaptureEvents} from './captureEvents.js'
import {sendTo, serverEvent} from '../../test/eventStream.js'
import type {EventFields} from '../../test/eventStream.js'
import type {ServerEventType} from '../api.events.js'

vi.mock('../api.js', async (importOriginal) =>
    (await import('../../test/mockApi.js')).mockApi(await importOriginal()))

import {dismissRecoveryFailure} from '../api.js'

beforeEach(() => vi.clearAllMocks())

/** A capture table, and `send(event)` to hand it an event as the stream would. */
function captureEvents() {
    const events = createCaptureEvents()
    const send = <K extends ServerEventType>(event: EventFields<K>) => sendTo(events.handlers, event.type, event)
    return {state: events.state, send}
}

const COUNTS: EventFields<'capture_counts'> = {
    type: 'capture_counts',
    frame_count: 120,
    stacked_count: 100,
    rejected_count: 20,
    dropped_count: 4,
    delivered_count: 124,
    last_rejection_reason: 'trailed stars',
}

describe('captureEvents sync snapshot', () => {
    // A page opened between captures had no counts at all, and one that missed a
    // `frame_dropped` kept the old drop count for the rest of the session.
    it('takes every counter and the last rejection reason from capture_counts', () => {
        const {state, send} = captureEvents()

        send(COUNTS)

        expect([
            state.frameCount.value, state.stackedCount.value, state.rejectedCount.value,
            state.droppedCount.value, state.deliveredCount.value, state.lastRejectionReason.value,
        ]).toEqual([120, 100, 20, 4, 124, 'trailed stars'])
    })

    it('drops a held rejection reason the server no longer has', () => {
        const {state, send} = captureEvents()
        send({type: 'frame_captured', frame_number: 3, stacked_count: 2, rejected_count: 1, rejection_reason: 'soft'})

        send({...COUNTS, frame_count: 0, stacked_count: 0, rejected_count: 0, last_rejection_reason: null})

        expect(state.lastRejectionReason.value).toBeNull()
        expect(state.frameCount.value).toBe(0)
    })

    // The snapshot sends the state first: a capture starting zeroes the counts, then the
    // counts the server holds land on top.
    it('keeps the restated counts after a snapshot state of Starting', () => {
        const {state, send} = captureEvents()

        send({type: 'state_changed', state: 'Starting'})
        send({...COUNTS, frame_count: 2, stacked_count: 2, rejected_count: 0})

        expect(state.frameCount.value).toBe(2)
    })

    it('restates the slow-disk warning, and its absence', () => {
        const {state, send} = captureEvents()

        send({type: 'disk_writer_warning', queue_depth: 40})
        expect(state.diskWriterWarning.value).toBe(40)
        send({type: 'disk_writer_warning_cleared'})
        expect(state.diskWriterWarning.value).toBeNull()
    })
})

describe('captureEvents camera recovery', () => {
    it('takes a reconnect notice down when the snapshot says that role has none', () => {
        const {state, send} = captureEvents()
        send({type: 'camera_reconnecting', name: 'Ares', role: 'main', attempt: 3, of: 9, next_attempt_in_s: 5})

        send({type: 'camera_recovery_cleared', role: 'guide'})
        expect(state.unresponsiveWarning.value).toContain('Reconnecting to Ares')

        send({type: 'camera_recovery_cleared', role: 'main'})
        expect(state.unresponsiveWarning.value).toBeNull()
    })

    // A lost give-up left "Reconnecting" up, hiding the reason recovery failed.
    it('replaces a stale reconnect notice with the give-up the snapshot restates', () => {
        const {state, send} = captureEvents()
        send({type: 'camera_reconnecting', name: 'Ares', role: 'main', attempt: 3, of: 9, next_attempt_in_s: 5})

        send({type: 'camera_reconnect_failed', name: 'Ares', role: 'main', attempts: 9, reason: 'unplugged'})
        send({type: 'camera_recovery_cleared', role: 'guide'})

        expect(state.unresponsiveWarning.value).toBe('Could not bring Ares back after 9 attempts: unplugged.')
        send({type: 'camera_recovery_cleared', role: 'main'})
        expect(state.unresponsiveWarning.value).toBeNull()
    })

    // The clear speaks for recovery only; a watchdog's warning waits for the camera.
    it('keeps an unresponsive-camera warning through a recovery clear', () => {
        const {state, send} = captureEvents()
        send({type: 'camera_persistently_unresponsive', name: 'Ares', consecutive_timeouts: 3})

        send({type: 'camera_recovery_cleared', role: 'main'})

        expect(state.unresponsiveWarning.value).toBe('Ares has stopped responding.')
    })

    it('clears the warning when the camera_phases snapshot shows that camera healthy', () => {
        const {state, send} = captureEvents()
        send({type: 'camera_persistently_unresponsive', name: 'Ares', consecutive_timeouts: 3})

        send({type: 'camera_phases', cameras: [{name: 'Ares', role: 'main', phase: 'recovering'}]})
        expect(state.unresponsiveWarning.value).not.toBeNull()

        send({type: 'camera_phases', cameras: [{name: 'Ares', role: 'main', phase: 'capturing'}]})
        expect(state.unresponsiveWarning.value).toBeNull()
    })

    it('keeps the warning when only a twin in the other role is healthy', () => {
        const {state, send} = captureEvents()
        send({type: 'camera_reconnecting', name: 'ASI120MM', role: 'guide', attempt: 2, of: 9, next_attempt_in_s: 3})

        send({type: 'camera_phases', cameras: [{name: 'ASI120MM', role: 'main', phase: 'idle'}]})

        expect(state.unresponsiveWarning.value).toContain('Reconnecting to ASI120MM')
    })

    // The server restates a give-up on every sync until a camera connects into the role:
    // dismissed only on the page, it came back after every reconnect, all night.
    it('dismisses a give-up on the server too, for every client', () => {
        const {state, send} = captureEvents()
        send({type: 'camera_reconnect_failed', name: 'Ares', role: 'guide', attempts: 9, reason: 'unplugged'})

        state.clearUnresponsiveWarning()

        expect(state.unresponsiveWarning.value).toBeNull()
        expect(dismissRecoveryFailure).toHaveBeenCalledExactlyOnceWith('guide')
    })

    it.each([
        ['a reconnect still running', serverEvent('camera_reconnecting', {name: 'Ares', role: 'main', attempt: 3, of: 9, next_attempt_in_s: 5})],
        ['a watchdog warning', serverEvent('camera_persistently_unresponsive', {name: 'Ares', consecutive_timeouts: 3})],
    ])('dismisses %s on this page only', (_, event) => {
        const {state, send} = captureEvents()
        send(event)

        state.clearUnresponsiveWarning()

        expect(state.unresponsiveWarning.value).toBeNull()
        expect(dismissRecoveryFailure).not.toHaveBeenCalled()
    })

    it('takes the warning down with the resume notice', () => {
        const {state, send} = captureEvents()
        send({type: 'camera_reconnecting', name: 'Ares', role: 'main', attempt: 3, of: 9, next_attempt_in_s: 5})

        send({type: 'capture_resumed', name: 'Ares', stacked_count: 51})
        send({type: 'camera_recovery_cleared', role: 'main'})

        expect(state.unresponsiveWarning.value).toBeNull()
        expect(state.resumeNotice.value).toContain('51 frames still stacked')
    })
})
