import {computed} from 'vue'
import {
    setTargetByName,
    setTargetByCoordinates,
    clearTarget as apiClearTarget,
    cancelPushToSolve as apiCancelSolve,
} from './api.js'
import {runDirectly} from './useError.js'

/**
 * Push-To target management over the event stream, which owns the target, the push
 * direction and the solve (`events/pushToEvents.js`): requests go to the server here, and
 * their answers are handed to the stream rather than written into its refs.
 * @param {Object} options
 * @param {Function} [options.withErrorHandling] - Error handling wrapper; errors propagate without one
 * @param {Object} options.eventStream - The provided `useEventStream()`
 * @returns Reactive target state and methods
 */
export function usePushToTarget({withErrorHandling = runDirectly, eventStream}) {
    const isSolving = computed(() => eventStream.plateSolving.value.inProgress)

    function selectTargetByName(designation) {
        return withErrorHandling(async () => eventStream.aimAt(await setTargetByName(designation)))
    }

    function selectTargetByCoordinates(ra, dec) {
        return withErrorHandling(async () => eventStream.aimAt(await setTargetByCoordinates(ra, dec)))
    }

    function clearTarget() {
        return withErrorHandling(async () => {
            await apiClearTarget()
            eventStream.forgetTarget()
        })
    }

    function cancelSolve() {
        return withErrorHandling(async () => {
            await apiCancelSolve()
            // The server also broadcasts `plate_solving_cancelled`, but clearing here
            // keeps the button from lingering for a round trip.
            eventStream.clearPlateSolving()
        })
    }

    return {
        currentTarget: eventStream.currentTarget,
        pushDirection: eventStream.pushDirection,
        isSolving,
        selectTargetByName,
        selectTargetByCoordinates,
        clearTarget,
        cancelSolve,
    }
}
