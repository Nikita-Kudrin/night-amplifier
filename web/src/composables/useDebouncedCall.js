import {tryOnBeforeUnmount} from './lifecycle.js'

/**
 * Calls `fn` `ms` after the last `schedule`, with that call's arguments.
 *
 * A pending call runs at once when the owning component unmounts: Vue drops an `emit` from
 * an unmounted component, so a debounced save whose panel closed within the delay was lost.
 */
export function useDebouncedCall(fn, ms) {
    let timer = null
    let pendingArgs = null

    function cancel() {
        clearTimeout(timer)
        timer = null
        pendingArgs = null
    }

    function flush() {
        if (!pendingArgs) return undefined
        const args = pendingArgs
        cancel()
        return fn(...args)
    }

    function schedule(...args) {
        clearTimeout(timer)
        pendingArgs = args
        timer = setTimeout(flush, ms)
    }

    tryOnBeforeUnmount(flush)

    return {schedule, flush, cancel}
}
