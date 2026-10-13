import {ref} from 'vue'

/**
 * Runs `operation`, resolving to its result — or to `undefined` where the handler took
 * care of a failure. What a component hands a composable that reports errors through it.
 */
export type ErrorHandler = <T>(operation: () => Promise<T>) => Promise<T | undefined>

/** The `withErrorHandling` of a caller handed none: runs the operation, errors propagate. */
export const runDirectly: ErrorHandler = (operation) => operation()

export interface ErrorHandlingOptions {
    /** Called with the error, after it is shown. */
    onError?: ((error: unknown) => void) | null
    /** Rejects with the error as well as showing it. */
    rethrow?: boolean
    /** Clears the shown error when the operation starts. */
    clearOnStart?: boolean
}

/** What a failure says: its message, else a generic one. */
function messageOf(e: unknown): string {
    const message = typeof e === 'object' && e !== null && 'message' in e ? e.message : undefined
    return typeof message === 'string' && message ? message : 'An error occurred'
}

/**
 * Composable for consistent error handling in components
 * Provides error state management and async operation wrapper
 */
export function useError() {
    const error = ref<string | null>(null)
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
    function setError(message: string | null) {
        error.value = message
    }

    /**
     * Wrap an async operation with loading state and error handling.
     * Resolves to the operation's result, or `undefined` on error.
     */
    async function withErrorHandling<T>(
        operation: () => Promise<T>,
        options: ErrorHandlingOptions = {},
    ): Promise<T | undefined> {
        const {onError = null, rethrow = false, clearOnStart = true} = options

        if (clearOnStart) {
            error.value = null
        }
        loading.value = true

        try {
            const result = await operation()
            return result
        } catch (e) {
            const message = messageOf(e)
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
