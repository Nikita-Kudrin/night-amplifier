import {tryOnBeforeUnmount} from './lifecycle'

/**
 * Calls `fn` `ms` after the last `schedule`, with that call's arguments.
 *
 * A pending call runs at once when the owning component unmounts: Vue drops an `emit` from
 * an unmounted component, so a debounced save whose panel closed within the delay was lost.
 */
export function useDebouncedCall<A extends unknown[], R>(fn: (...args: A) => R, ms: number) {
    let timer: ReturnType<typeof setTimeout> | undefined
    let pendingArgs: A | null = null

    function cancel() {
        clearTimeout(timer)
        timer = undefined
        pendingArgs = null
    }

    function flush(): R | undefined {
        if (!pendingArgs) return undefined
        const args = pendingArgs
        cancel()
        return fn(...args)
    }

    function schedule(...args: A) {
        clearTimeout(timer)
        pendingArgs = args
        timer = setTimeout(flush, ms)
    }

    tryOnBeforeUnmount(flush)

    return {schedule, flush, cancel}
}
