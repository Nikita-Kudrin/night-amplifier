import {ref} from 'vue'
import {dismissRecoveryFailure} from '../api.js'
import {CAMERA_PHASES} from '../../constants'

/**
 * What `/ws/events` says about the capture session: its state and counters, the faults
 * and notices it raises, and which camera the eyepiece pages show. Every ref is written
 * by `handlers` or by the `clear*` dismissals, nowhere else.
 */
export function createCaptureEvents() {
    const captureState = ref('Idle')
    const frameCount = ref(0)
    const stackedCount = ref(0)
    const rejectedCount = ref(0)
    const droppedCount = ref(0)
    // The denominator the count needs: 40 drops is a ruined evening at 30 s subs and a
    // rounding error at 100 ms.
    const deliveredCount = ref(0)
    const lastError = ref(null)
    const diskWriterWarning = ref(null)
    const unresponsiveWarning = ref(null)
    // Which camera the warning is about — `{name, role, recovery, gaveUp}`, role null when the
    // event had none, `recovery` when automatic recovery raised it (`gaveUp` its give-up) —
    // so its end can clear it.
    const unresponsiveCamera = ref(null)
    const resumeNotice = ref(null)
    // Why the server switched Focus/Finder mode off on its own; stays until dismissed.
    const focusModeNotice = ref(null)
    // The camera the eyepiece pages show, chosen by the Guide toggle on `/`. The server
    // sends it on connect, so a reloaded page shows the toggle where it was left.
    const viewedCamera = ref('main')
    // Counts `viewed_camera_changed`, so a PUT's answer can tell an event overtook it.
    const viewedCameraRevision = ref(0)
    // Where the main stream's frames were cut from the stack (`auto_crop`, `crop` null when
    // whole) and the live-stream frame size that cut arrives at (`frame`, null until known).
    const stackCrop = ref({crop: null, frame: null})
    // Why the most recent frame was kept out of the stack. Held rather than cleared per
    // frame, so the reason a rejection burst is happening stays readable while good
    // frames come and go between the bad ones.
    const lastRejectionReason = ref(null)

    function clearUnresponsive() {
        unresponsiveWarning.value = null
        unresponsiveCamera.value = null
    }

    /**
     * The observer dismissed the warning. A give-up is also dismissed on the server, which
     * restates it on every sync until a camera connects into the role: dismissed only here,
     * it came back after every reconnect, all night for one imaging on without its guide.
     */
    function dismissUnresponsive() {
        const warned = unresponsiveCamera.value
        clearUnresponsive()
        if (!warned?.gaveUp || !warned.role) return
        dismissRecoveryFailure(warned.role).catch((e) => console.warn('Could not dismiss the recovery warning:', e))
    }

    /**
     * A camera that came back needs no warning, and a guide camera or an idle one gets no
     * `capture_resumed` to clear it. Matched on role too where both sides carry one: two
     * bodies of one model share a name.
     */
    function clearIfHealthy({name, role, phase}) {
        const healthy = phase !== CAMERA_PHASES.RECOVERING && phase !== CAMERA_PHASES.DISCONNECTED
        const warned = unresponsiveCamera.value
        const sameCamera = warned?.name === name && (!warned.role || !role || warned.role === role)
        if (healthy && sameCamera) clearUnresponsive()
    }

    function handleFrameEvent(data) {
        frameCount.value = data.frame_number
        stackedCount.value = data.stacked_count
        rejectedCount.value = data.rejected_count
        if (data.rejection_reason) {
            lastRejectionReason.value = data.rejection_reason
        }
    }

    const handlers = {
        state_changed(data) {
            // A capture resuming after a camera reconnect carries its session on; only a
            // fresh start zeroes the counters.
            const resuming = captureState.value === 'Recovering'
            captureState.value = data.state
            if (data.state === 'Starting' && !resuming) {
                frameCount.value = 0
                stackedCount.value = 0
                rejectedCount.value = 0
                droppedCount.value = 0
                deliveredCount.value = 0
                lastRejectionReason.value = null
            }
        },
        viewed_camera_changed(data) {
            viewedCamera.value = data.camera
            viewedCameraRevision.value++
        },
        stack_crop_changed(data) {
            stackCrop.value = {crop: data.crop ?? null, frame: data.frame ?? null}
        },
        frame_captured: handleFrameEvent,
        frame_rejected: handleFrameEvent,
        error(data) {
            lastError.value = data.message
        },
        camera_persistently_unresponsive(data) {
            // The server sends `name`, not `camera_name` — see the shape pinned
            // by src/server/tests/events.rs.
            unresponsiveWarning.value = `${data.name} has stopped responding.`
            unresponsiveCamera.value = {name: data.name, role: null}
        },
        camera_reconnecting(data) {
            unresponsiveWarning.value =
                `Reconnecting to ${data.name} — attempt ${data.attempt} of ${data.of}.`
            unresponsiveCamera.value = {name: data.name, role: data.role ?? null, recovery: true}
        },
        camera_phase_changed: clearIfHealthy,
        // The sync's restatement of every phase, which a page that missed the healthy
        // `camera_phase_changed` would otherwise never see.
        camera_phases(data) {
            for (const camera of data.cameras ?? []) clearIfHealthy(camera)
        },
        camera_reconnect_failed(data) {
            unresponsiveWarning.value =
                `Could not bring ${data.name} back after ${data.attempts} attempts: ${data.reason}.`
            unresponsiveCamera.value = {name: data.name, role: data.role ?? null, recovery: true, gaveUp: true}
        },
        // Sent for every role on each sync: only a warning recovery raised for that role goes.
        camera_recovery_cleared(data) {
            const warned = unresponsiveCamera.value
            if (warned?.recovery && warned.role === data.role) clearUnresponsive()
        },
        capture_resumed(data) {
            clearUnresponsive()
            lastError.value = null
            resumeNotice.value =
                `${data.name} is back. Capture resumed with ${data.stacked_count} frames still stacked.`
        },
        focus_mode_left() {
            focusModeNotice.value =
                'Focus/Finder mode turned off: stacking needs the sensor corrections it holds off.'
        },
        disk_writer_warning(data) {
            diskWriterWarning.value = data.queue_depth
        },
        disk_writer_warning_cleared() {
            diskWriterWarning.value = null
        },
        frame_dropped(data) {
            droppedCount.value = data.dropped_count
            deliveredCount.value = data.delivered_count ?? 0
        },
        // The sync's restatement of every counter: a page opened between captures shows the
        // last session's, and one that missed a `frame_dropped` catches up.
        capture_counts(data) {
            frameCount.value = data.frame_count
            stackedCount.value = data.stacked_count
            rejectedCount.value = data.rejected_count
            droppedCount.value = data.dropped_count
            deliveredCount.value = data.delivered_count
            lastRejectionReason.value = data.last_rejection_reason ?? null
        },
    }

    return {
        handlers,
        state: {
            captureState,
            frameCount,
            stackedCount,
            rejectedCount,
            lastRejectionReason,
            droppedCount,
            deliveredCount,
            lastError,
            diskWriterWarning,
            unresponsiveWarning,
            resumeNotice,
            focusModeNotice,
            viewedCamera,
            stackCrop,
            viewedCameraRevision,
            clearError: () => (lastError.value = null),
            clearDiskWriterWarning: () => (diskWriterWarning.value = null),
            clearUnresponsiveWarning: dismissUnresponsive,
            clearResumeNotice: () => (resumeNotice.value = null),
            clearFocusModeNotice: () => (focusModeNotice.value = null),
        },
    }
}
