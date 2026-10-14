import {ref} from 'vue'
import {dismissRecoveryFailure} from '../api'
import {CAMERA_PHASES} from '../../constants'
import type {CameraRole, CaptureState} from '../api.types'
import type {
    CameraPhaseEntry,
    FrameCapturedEvent,
    FrameRejectedEvent,
    StreamCrop,
    StreamFrameSize,
} from '../api.events'
import type {EventHandlers} from './handlers'

/**
 * The camera a recovery or watchdog warning is about: `role` null when the event had none,
 * `recovery` when automatic recovery raised it, `gaveUp` its give-up.
 */
interface WarnedCamera {
    name: string
    role: CameraRole | null
    recovery?: boolean
    gaveUp?: boolean
}

/** Where the main stream's frames were cut from the stack, and the frame size the cut arrives at. */
export interface StackCrop {
    crop: StreamCrop | null
    frame: StreamFrameSize | null
}

/**
 * What `/ws/events` says about the capture session: its state and counters, the faults
 * and notices it raises, and which camera the eyepiece pages show. Every ref is written
 * by `handlers` or by the `clear*` dismissals, nowhere else.
 */
export function createCaptureEvents() {
    const captureState = ref<CaptureState>('Idle')
    const frameCount = ref(0)
    const stackedCount = ref(0)
    const rejectedCount = ref(0)
    const droppedCount = ref(0)
    // The denominator the count needs: 40 drops is a ruined evening at 30 s subs and a
    // rounding error at 100 ms.
    const deliveredCount = ref(0)
    const lastError = ref<string | null>(null)
    const diskWriterWarning = ref<number | null>(null)
    const unresponsiveWarning = ref<string | null>(null)
    // Which camera the warning is about, so its end can clear it.
    const unresponsiveCamera = ref<WarnedCamera | null>(null)
    const resumeNotice = ref<string | null>(null)
    // Why the server switched Focus/Finder mode off on its own; stays until dismissed.
    const focusModeNotice = ref<string | null>(null)
    // The camera the eyepiece pages show, chosen by the Guide toggle on `/`. The server
    // sends it on connect, so a reloaded page shows the toggle where it was left.
    const viewedCamera = ref<CameraRole>('main')
    // Counts `viewed_camera_changed`, so a PUT's answer can tell an event overtook it.
    const viewedCameraRevision = ref(0)
    // `auto_crop`'s cut: `crop` null when whole, `frame` null until known.
    const stackCrop = ref<StackCrop>({crop: null, frame: null})
    // Why the most recent frame was kept out of the stack. Held rather than cleared per
    // frame, so the reason a rejection burst is happening stays readable while good
    // frames come and go between the bad ones.
    const lastRejectionReason = ref<string | null>(null)

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
    function clearIfHealthy({name, role, phase}: Pick<CameraPhaseEntry, 'name' | 'role' | 'phase'>) {
        const healthy = phase !== CAMERA_PHASES.RECOVERING && phase !== CAMERA_PHASES.DISCONNECTED
        const warned = unresponsiveCamera.value
        const sameCamera = warned?.name === name && (!warned.role || !role || warned.role === role)
        if (healthy && sameCamera) clearUnresponsive()
    }

    function handleFrameEvent(data: FrameCapturedEvent | FrameRejectedEvent) {
        frameCount.value = data.frame_number
        stackedCount.value = data.stacked_count
        rejectedCount.value = data.rejected_count
        if ('rejection_reason' in data && data.rejection_reason) {
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
            unresponsiveCamera.value = {name: data.name, role: data.role, recovery: true}
        },
        camera_phase_changed: clearIfHealthy,
        // The sync's restatement of every phase, which a page that missed the healthy
        // `camera_phase_changed` would otherwise never see.
        camera_phases(data) {
            for (const camera of data.cameras) clearIfHealthy(camera)
        },
        camera_reconnect_failed(data) {
            unresponsiveWarning.value =
                `Could not bring ${data.name} back after ${data.attempts} attempts: ${data.reason}.`
            unresponsiveCamera.value = {name: data.name, role: data.role, recovery: true, gaveUp: true}
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
            deliveredCount.value = data.delivered_count
        },
        // The sync's restatement of every counter: a page opened between captures shows the
        // last session's, and one that missed a `frame_dropped` catches up.
        capture_counts(data) {
            frameCount.value = data.frame_count
            stackedCount.value = data.stacked_count
            rejectedCount.value = data.rejected_count
            droppedCount.value = data.dropped_count
            deliveredCount.value = data.delivered_count
            lastRejectionReason.value = data.last_rejection_reason
        },
    } satisfies EventHandlers

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
