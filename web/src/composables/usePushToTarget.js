import {computed, ref, onMounted} from 'vue'
import {
    getPushToStatus,
    setTargetByName,
    setTargetByCoordinates,
    clearTarget as apiClearTarget,
    cancelPushToSolve as apiCancelSolve,
    ignoreProOnly,
} from './api.js'
import {pushDirectionFrom} from './useEventStream.js'
import {runDirectly} from './useError.js'

/**
 * Composable for Push-To target management
 * @param {Object} options - Configuration options
 * @param {Function} [options.withErrorHandling] - Error handling wrapper; errors propagate without one
 * @param {Object} options.eventStream - Optional event stream to sync with
 * @returns Reactive target state and methods
 */
export function usePushToTarget({withErrorHandling = runDirectly, eventStream} = {}) {
    // If eventStream is provided, use its refs to ensure synchronization
    // Otherwise, create local refs (backward compatibility or standalone use)
    const currentTarget = eventStream?.currentTarget || ref(null)
    const pushDirection = eventStream?.pushDirection || ref(null)
    const currentPosition = ref(null)

    // Seeded from the status poll on mount, then owned by the event stream. As a
    // plain ref it was only ever written by that one poll and by cancelSolve(), so
    // it was false for the entire life of the page: the panel's spinner and its
    // Cancel button never appeared, no matter how long a solve ran.
    const solvingAtMount = ref(false)
    const isSolving = eventStream?.plateSolving
        ? computed(() => eventStream.plateSolving.value?.inProgress ?? solvingAtMount.value)
        : solvingAtMount

    async function refreshStatus() {
        try {
            const status = await getPushToStatus()
            // Backend returns current_target (snake_case)
            currentTarget.value = status.current_target
            currentPosition.value = status.last_position
            pushDirection.value = status.direction ? pushDirectionFrom(status.direction) : null
            solvingAtMount.value = status.is_solving
        } catch (e) {
            ignoreProOnly(e, 'Push-To status')
        }
    }

    /** Takes the target the server answers with; its direction follows on the event stream. */
    function aimAt(target) {
        currentTarget.value = target
        pushDirection.value = null
        return target
    }

    function selectTargetByName(designation) {
        return withErrorHandling(async () => aimAt(await setTargetByName(designation)))
    }

    function selectTargetByCoordinates(ra, dec) {
        return withErrorHandling(async () => aimAt(await setTargetByCoordinates(ra, dec)))
    }

    function clearTarget() {
        return withErrorHandling(async () => {
            await apiClearTarget()
            currentTarget.value = null
            pushDirection.value = null
            eventStream?.clearPlateSolving?.()
        })
    }

    function cancelSolve() {
        return withErrorHandling(async () => {
            await apiCancelSolve()
            solvingAtMount.value = false
            // The server also broadcasts `plate_solving_cancelled`, but clearing here
            // keeps the button from lingering for a round trip.
            eventStream?.clearPlateSolving?.()
        })
    }

    onMounted(() => {
        refreshStatus()
    })

    return {
        currentTarget,
        currentPosition,
        pushDirection,
        isSolving,
        refreshStatus,
        selectTargetByName,
        selectTargetByCoordinates,
        clearTarget,
        cancelSolve,
    }
}
