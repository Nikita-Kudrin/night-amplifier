import {onScopeDispose} from 'vue'

/**
 * `setTimeout` owned by the calling component (or effect scope): whatever is still
 * pending when it goes away is cleared, so a callback never runs against a torn-down view.
 */
export function useTimeouts() {
    const pending = new Set()

    function later(callback, ms) {
        const id = setTimeout(() => {
            pending.delete(id)
            callback()
        }, ms)
        pending.add(id)
        return id
    }

    function clearAll() {
        pending.forEach(clearTimeout)
        pending.clear()
    }

    onScopeDispose(clearAll)

    return {later, clearAll}
}
