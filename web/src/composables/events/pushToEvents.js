import {computed, ref} from 'vue'
import {getPushToStatus, ignoreProOnly} from '../api.js'

const NOT_SOLVING = Object.freeze({inProgress: false, targetName: null, lastResult: null, stage: null})

/**
 * The client's push direction from a `push_direction_updated` event or the REST
 * status' `direction`; only the event carries `fov_deg`.
 * @param {object} direction - Snake-case direction from the server
 * @returns {{angleDeg: number, distanceDeg: number, directionHint: string, isClose: boolean, fovDeg: number, solvedCamera: string|null}}
 */
export function pushDirectionFrom(direction) {
    return {
        angleDeg: direction.angle_deg,
        distanceDeg: direction.distance_deg,
        directionHint: direction.direction_hint,
        isClose: direction.is_close,
        fovDeg: direction.fov_deg || 0,
        solvedCamera: direction.solved_camera ?? null,
    }
}

/**
 * Push-To's target, direction and solve, owned here alone: the events, the status read on
 * every socket open, and the panel's own answers (`aimAt`, `forgetTarget`) all write
 * through this module. The panel once wrote these refs itself and refetched the status on
 * mount, so three writers raced and a stale status could put a cleared target back.
 */
export function createPushToEvents() {
    const pushDirection = ref(null)
    const currentTarget = ref(null)
    const plateSolving = ref({...NOT_SOLVING})
    // Why Push-To is idle, when it is idle for a reason the user can act on (no target,
    // ASTAP not installed, waiting out a retry backoff). The server sends this only on a
    // transition, so it is safe to render directly.
    const pushToBlocked = ref(null)
    // Bumped by every write, so a status answer that left the server before one is dropped.
    let revision = 0

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

    function finishSolve(lastResult) {
        plateSolving.value = {inProgress: false, targetName: plateSolving.value.targetName, lastResult, stage: null}
    }

    function clearPlateSolving() {
        plateSolving.value = {...NOT_SOLVING}
        pushToBlocked.value = null
    }

    function clearPushDirection() {
        pushDirection.value = null
    }

    function aimAt(target) {
        revision++
        currentTarget.value = target
        pushDirection.value = null
        return target
    }

    function forgetTarget() {
        revision++
        currentTarget.value = null
        pushDirection.value = null
        clearPlateSolving()
    }

    /** The server's Push-To state, read when the socket opens: no event resends it. */
    async function loadStatus() {
        const asked = revision
        let status
        try {
            status = await getPushToStatus()
        } catch (e) {
            ignoreProOnly(e, 'Push-To status')
            return
        }
        if (revision !== asked) return
        currentTarget.value = status.current_target || null
        pushDirection.value = status.direction ? pushDirectionFrom(status.direction) : null
        if (status.is_solving) plateSolving.value = {...plateSolving.value, inProgress: true}
    }

    // Each wrapped so it bumps `revision`; see `loadStatus`.
    const handlers = Object.fromEntries(Object.entries({
        plate_solving_started(data) {
            plateSolving.value = {inProgress: true, targetName: data.target_name, lastResult: null, stage: null}
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
        position_solved: () => finishSolve('success'),
        position_solve_failed: () => finishSolve('failed'),
        // Distinct from a failure: the user asked for this, and the last known position
        // is still good. Showing it as "Failed to find M31" made an ordinary settings
        // save look like a solver problem.
        plate_solving_cancelled: () => finishSolve('cancelled'),
        plate_solving_restarted() {
            finishSolve(null)
            pushToBlocked.value = null
        },
        push_to_blocked(data) {
            pushToBlocked.value = data.reason ?? null
        },
        push_direction_updated(data) {
            pushDirection.value = pushDirectionFrom(data)
        },
        target_changed(data) {
            currentTarget.value = {
                name: data.name,
                designation: data.designation,
                ra_degrees: data.ra_degrees,
                dec_degrees: data.dec_degrees,
            }
        },
        target_cleared: forgetTarget,
    }).map(([type, handle]) => [type, (data) => {
        revision++
        handle(data)
    }]))

    return {
        handlers,
        loadStatus,
        state: {
            pushDirection,
            currentTarget,
            plateSolving,
            pushToBlocked,
            solvingMessage,
            aimAt,
            forgetTarget,
            clearPushDirection,
            clearPlateSolving,
        },
    }
}
