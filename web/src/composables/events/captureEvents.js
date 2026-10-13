import {ref} from 'vue'
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
    // Which camera the warning is about — `{name, role}`, role null when the event had
    // none — so its recovery can clear it.
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
            unresponsiveCamera.value = {name: data.name, role: data.role ?? null}
        },
        camera_phase_changed(data) {
            // A camera that came back needs no warning, and a guide camera or an idle
            // one gets no `capture_resumed` to clear it. Matched on role too where both
            // sides carry one: two bodies of one model share a name.
            const healthy = data.phase !== CAMERA_PHASES.RECOVERING && data.phase !== CAMERA_PHASES.DISCONNECTED
            const warned = unresponsiveCamera.value
            const sameCamera = warned?.name === data.name
                && (!warned.role || !data.role || warned.role === data.role)
            if (healthy && sameCamera) {
                unresponsiveWarning.value = null
                unresponsiveCamera.value = null
            }
        },
        camera_reconnect_failed(data) {
            unresponsiveWarning.value =
                `Could not bring ${data.name} back after ${data.attempts} attempts: ${data.reason}.`
        },
        capture_resumed(data) {
            unresponsiveWarning.value = null
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
            clearUnresponsiveWarning: () => (unresponsiveWarning.value = null),
            clearResumeNotice: () => (resumeNotice.value = null),
            clearFocusModeNotice: () => (focusModeNotice.value = null),
        },
    }
}
