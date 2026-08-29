import {computed, ref, onMounted} from 'vue'
import {
    getPushToStatus,
    setTargetByName,
    setTargetByCoordinates,
    clearTarget as apiClearTarget,
    cancelPushToSolve as apiCancelSolve,
} from './api.js'

/**
 * Composable for Push-To target management
 * @param {Object} options - Configuration options
 * @param {Function} options.withErrorHandling - Error handling wrapper function
 * @param {Object} options.eventStream - Optional event stream to sync with
 * @returns Reactive target state and methods
 */
export function usePushToTarget({withErrorHandling, eventStream} = {}) {
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
            solvingAtMount.value = status.is_solving
        } catch {
            // Ignore - push-to may not be initialized
        }
    }

    async function selectTargetByName(designation) {
        const execute = async () => {
            const result = await setTargetByName(designation)
            // api.setTargetByName returns the target object directly
            currentTarget.value = result
            pushDirection.value = null
            return result
        }

        if (withErrorHandling) {
            return withErrorHandling(execute)
        }
        return execute()
    }

    async function selectTargetByCoordinates(ra, dec) {
        const execute = async () => {
            const result = await setTargetByCoordinates(ra, dec)
            // api.setTargetByCoordinates returns the target object directly
            currentTarget.value = result
            pushDirection.value = null
            return result
        }

        if (withErrorHandling) {
            return withErrorHandling(execute)
        }
        return execute()
    }

    async function clearTarget() {
        const execute = async () => {
            await apiClearTarget()
            currentTarget.value = null
            pushDirection.value = null
            if (eventStream?.clearPlateSolving) {
                eventStream.clearPlateSolving()
            }
        }

        if (withErrorHandling) {
            return withErrorHandling(execute)
        }
        return execute()
    }

    async function cancelSolve() {
        const execute = async () => {
            await apiCancelSolve()
            solvingAtMount.value = false
            // The server also broadcasts `plate_solving_cancelled`, but clearing here
            // keeps the button from lingering for a round trip.
            if (eventStream?.clearPlateSolving) {
                eventStream.clearPlateSolving()
            }
        }

        if (withErrorHandling) {
            return withErrorHandling(execute)
        }
        return execute()
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
