import {ref} from 'vue'

/**
 * Composable for consistent error handling in components
 * Provides error state management and async operation wrapper
 */
export function useError() {
    const error = ref(null)
    const loading = ref(false)

    /**
     * Clear the current error
     */
    function clearError() {
        error.value = null
    }

    /**
     * Set an error message
     */
    function setError(message) {
        error.value = message
    }

    /**
     * Wrap an async operation with loading state and error handling
     * @param {Function} operation - Async function to execute
     * @param {Object} options - Options for the operation
     * @returns {Promise<any>} Result of the operation or undefined on error
     */
    async function withErrorHandling(operation, options = {}) {
        const {onError = null, rethrow = false, clearOnStart = true} = options

        if (clearOnStart) {
            error.value = null
        }
        loading.value = true

        try {
            const result = await operation()
            return result
        } catch (e) {
            const message = e.message || 'An error occurred'
            error.value = message

            if (onError) {
                onError(e)
            }

            if (rethrow) {
                throw e
            }

            return undefined
        } finally {
            loading.value = false
        }
    }

    return {
        error,
        loading,
        clearError,
        setError,
        withErrorHandling,
    }
}
