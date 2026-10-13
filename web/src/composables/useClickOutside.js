import {unref} from 'vue'
import {useEventListener} from './useEventListener.js'

/**
 * Calls `onOutside(event)` for a click anywhere but inside `targets` — elements or refs
 * to them (a teleported menu is its own target). The listener goes with the component.
 * @param {Array<Element|import('vue').Ref<Element|null>>} targets
 */
export function useClickOutside(targets, onOutside) {
    return useEventListener(document, 'click', (event) => {
        const inside = targets.some((target) => unref(target)?.contains(event.target))
        if (!inside) onOutside(event)
    })
}
