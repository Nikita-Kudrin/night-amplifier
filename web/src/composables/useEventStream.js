import {ref, computed} from 'vue'
import {getPushToStatus} from './api.js'
import {useWebSocket} from './useWebSocket.js'

/**
 * WebSocket composable for server events
 * @returns {object} Event stream state and methods
 */
export function useEventStream() {
    const lastEvent = ref(null)
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

    // Push-To state
    const pushDirection = ref(null)
    const currentTarget = ref(null)
    const plateSolving = ref({inProgress: false, targetName: null, lastResult: null, stage: null})

    // Why Push-To is idle, when it is idle for a reason the user can act on
    // (no target, ASTAP not installed, waiting out a retry backoff). The server
    // sends this only on a transition, so it is safe to render directly.
    const pushToBlocked = ref(null)

    const solvingMessage = computed(() => {
        const {inProgress, lastResult, targetName, stage} = plateSolving.value
        const targetSuffix = targetName ? `: ${targetName}` : ''

        if (inProgress) {
            // The step counter, not the strategy behind it. A label like "last solve
            // (this session) + last known pointing" is a diagnostic that overran the
            // status bar and pushed the one thing the user is looking for -- the object
            // name -- off the end. It stays in `stage.label` and in the server log.
            // The counter alone still separates a slow solve from a hung one.
            if (stage && stage.total > 1) {
                return `Searching (step ${stage.attempt}/${stage.total})${targetSuffix}`
            }
            return targetName ? `Searching${targetSuffix}` : 'Updating position...'
        }

        // A live blocker outranks the last verdict, because it is *newer*. The server
        // sends one only on a transition, so its presence means the situation has
        // moved on since that verdict -- the scope is being pushed, the view has not
        // settled, a retry is being held off. Ordering these the other way round is
        // what left "Found: M31" on screen for the whole time the user was pushing
        // away from M31, and hid the retry countdown behind "Failed to find M31"
        // forever, since nothing ever cleared `lastResult`.
        if (pushToBlocked.value) {
            return pushToBlocked.value
        }

        if (lastResult === 'success') {
            return targetName ? `Found${targetSuffix}` : 'Position updated'
        }

        if (lastResult === 'failed') {
            return targetName ? `Failed to find${targetSuffix}` : 'Update failed'
        }

        if (lastResult === 'cancelled') {
            return 'Solving cancelled'
        }

        return null
    })

    // ASTAP installation state
    const astapInstallProgress = ref(null)

    // Catalog installation state
    const catalogInstallProgress = ref(null)

    // Why the most recent frame was kept out of the stack. Held rather than
    // cleared per frame, so the reason a rejection burst is happening stays
    // readable while good frames come and go between the bad ones.
    const lastRejectionReason = ref(null)

    function handleFrameEvent(data) {
        frameCount.value = data.frame_number
        stackedCount.value = data.stacked_count
        rejectedCount.value = data.rejected_count
        if (data.rejection_reason) {
            lastRejectionReason.value = data.rejection_reason
        }
    }

    const eventHandlers = {
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
            const healthy = data.phase !== 'recovering' && data.phase !== 'disconnected'
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
        settings_updated() { /* components should refresh */
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
        plate_solving_started(data) {
            plateSolving.value = {
                inProgress: true, targetName: data.target_name, lastResult: null, stage: null,
            }
            // Solving has resumed, so whatever was holding it up no longer is.
            pushToBlocked.value = null
        },
        plate_solving_progress(data) {
            // A cold solve works down several strategies and the last one can run for
            // a minute. Without the stage the indicator is indistinguishable from a hang.
            plateSolving.value = {
                ...plateSolving.value,
                inProgress: true,
                stage: {label: data.stage, attempt: data.attempt, total: data.total},
            }
        },
        position_solved() {
            plateSolving.value = {
                inProgress: false, targetName: plateSolving.value.targetName,
                lastResult: 'success', stage: null,
            }
        },
        position_solve_failed() {
            plateSolving.value = {
                inProgress: false, targetName: plateSolving.value.targetName,
                lastResult: 'failed', stage: null,
            }
        },
        plate_solving_cancelled() {
            // Distinct from a failure: the user asked for this, and the last known
            // position is still good. Showing it as "Failed to find M31" made an
            // ordinary settings save look like a solver problem.
            plateSolving.value = {
                inProgress: false, targetName: plateSolving.value.targetName,
                lastResult: 'cancelled', stage: null,
            }
        },
        plate_solving_restarted() {
            plateSolving.value = {
                inProgress: false, targetName: plateSolving.value.targetName,
                lastResult: null, stage: null,
            }
            pushToBlocked.value = null
        },
        push_to_blocked(data) {
            pushToBlocked.value = data.reason ?? null
        },
        push_direction_updated(data) {
            pushDirection.value = {
                angleDeg: data.angle_deg,
                distanceDeg: data.distance_deg,
                directionHint: data.direction_hint,
                isClose: data.is_close,
                fovDeg: data.fov_deg || 0,
            }
        },
        target_changed(data) {
            currentTarget.value = {
                name: data.name,
                designation: data.designation,
                ra_degrees: data.ra_degrees,
                dec_degrees: data.dec_degrees,
            }
        },
        target_cleared() {
            currentTarget.value = null
            pushDirection.value = null
            clearPlateSolving()
        },
        astap_install_starting(data) {
            astapInstallProgress.value = {
                component: data.component, stage: 'starting',
                percent: null, bytesDownloaded: 0, totalBytes: null,
                overallPercent: null, error: null,
            }
        },
        astap_install_progress(data) {
            astapInstallProgress.value = {
                component: data.component, stage: 'downloading',
                percent: data.percent, bytesDownloaded: data.bytes_downloaded,
                totalBytes: data.total_bytes, stageName: data.stage,
                overallPercent: data.overall_percent, error: null,
            }
        },
        astap_install_extracting(data) {
            astapInstallProgress.value = {
                component: data.component, stage: 'extracting',
                percent: data.progress, stageName: data.stage,
                overallPercent: data.overall_percent, error: null,
            }
        },
        astap_install_completed(data) {
            astapInstallProgress.value = {
                component: data.component, stage: 'completed',
                stageName: data.stage, overallPercent: data.overall_percent, error: null,
            }
        },
        astap_install_failed(data) {
            astapInstallProgress.value = {component: data.component, stage: 'failed', error: data.error}
        },
        catalog_install_starting() {
            catalogInstallProgress.value = {
                stage: 'starting', fileName: '',
                percent: null, bytesDownloaded: 0, totalBytes: null, error: null,
            }
        },
        catalog_install_progress(data) {
            catalogInstallProgress.value = {
                stage: 'downloading', fileName: data.file_name,
                percent: data.percent, bytesDownloaded: data.bytes_downloaded,
                totalBytes: data.total_bytes, error: null,
            }
        },
        catalog_file_completed(data) {
            catalogInstallProgress.value = {
                ...catalogInstallProgress.value,
                fileName: data.file_name, stage: 'file_completed',
            }
        },
        catalog_install_completed(data) {
            catalogInstallProgress.value = {stage: 'completed', object_count: data.object_count, error: null}
        },
        catalog_install_failed(data) {
            catalogInstallProgress.value = {stage: 'failed', error: data.error}
        },
    }

    const {connected, error, connect, disconnect} = useWebSocket('/ws/events', {
        onOpen: async () => {
            try {
                // Fetch the initial state upon connection
                const status = await getPushToStatus()
                currentTarget.value = status.current_target || null
                if (status.direction) {
                    pushDirection.value = {
                        angleDeg: status.direction.angle_deg,
                        distanceDeg: status.direction.distance_deg,
                        directionHint: status.direction.direction_hint,
                        isClose: status.direction.is_close,
                        fovDeg: status.direction.fov_deg || 0,
                    }
                } else {
                    pushDirection.value = null
                }
            } catch {
                // Ignore
            }
        },
        onMessage: (event) => {
            try {
                const data = JSON.parse(event.data)
                lastEvent.value = data
                eventHandlers[data.type]?.(data)
            } catch (e) {
                console.error('Failed to parse event:', e)
            }
        },
    })

    function clearError() {
        lastError.value = null
    }

    function clearDiskWriterWarning() {
        diskWriterWarning.value = null
    }

    function clearPushDirection() {
        pushDirection.value = null
    }

    function clearPlateSolving() {
        plateSolving.value = {inProgress: false, targetName: null, lastResult: null, stage: null}
        pushToBlocked.value = null
    }

    function clearAstapInstallProgress() {
        astapInstallProgress.value = null
    }

    function clearCatalogInstallProgress() {
        catalogInstallProgress.value = null
    }

    function clearUnresponsiveWarning() {
        unresponsiveWarning.value = null
    }

    function clearResumeNotice() {
        resumeNotice.value = null
    }

    function clearFocusModeNotice() {
        focusModeNotice.value = null
    }

    return {
        connected,
        error,
        lastEvent,
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
        pushDirection,
        currentTarget,
        plateSolving,
        pushToBlocked,
        solvingMessage,
        astapInstallProgress,
        catalogInstallProgress,
        clearError,
        clearDiskWriterWarning,
        clearUnresponsiveWarning,
        clearResumeNotice,
        clearFocusModeNotice,
        clearPushDirection,
        clearPlateSolving,
        clearAstapInstallProgress,
        clearCatalogInstallProgress,
        connect,
        disconnect,
    }
}
