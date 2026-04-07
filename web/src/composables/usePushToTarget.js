import {ref, onMounted} from 'vue'
import {
    getPushToStatus,
    setTargetByName,
    setTargetByCoordinates,
    clearTarget as apiClearTarget,
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

    async function refreshStatus() {
        try {
            const status = await getPushToStatus()
            // Backend returns current_target (snake_case)
            currentTarget.value = status.current_target
            currentPosition.value = status.current_position
            pushDirection.value = status.direction
        } catch {
            // Ignore - push-to may not be initialized
        }
    }

    async function selectTargetByName(designation) {
        const execute = async () => {
            const result = await setTargetByName(designation)
            // api.setTargetByName returns the target object directly
            currentTarget.value = result
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
        refreshStatus,
        selectTargetByName,
        selectTargetByCoordinates,
        clearTarget,
    }
}
