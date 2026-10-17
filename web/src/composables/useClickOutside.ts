import {unref} from 'vue'
import type {MaybeRef} from 'vue'
import {useEventListener} from './useEventListener'

/**
 * Calls `onOutside(event)` for a click anywhere but inside `targets` — elements or refs
 * to them (a teleported menu is its own target). The listener goes with the component.
 */
export function useClickOutside(
    targets: readonly MaybeRef<Element | null | undefined>[],
    onOutside: (event: MouseEvent) => void,
) {
    return useEventListener(document, 'click', (event) => {
        const inside = targets.some((target) => unref(target)?.contains(event.target as Node | null))
        if (!inside) onOutside(event)
    })
}
