import {readonly, ref, toValue, watch} from 'vue'
import type {MaybeRefOrGetter, Ref} from 'vue'
import {useTimeouts} from './useTimeouts'

/**
 * `source`, held true for `holdMs` after it falls, so a state that is over within a blink
 * still reads as one rather than a flicker. A rise during the hold keeps it up; the hold
 * is cleared with the calling component.
 */
export function useHeldFlag(source: MaybeRefOrGetter<boolean>, holdMs: number): Readonly<Ref<boolean>> {
    const held = ref(toValue(source))
    const {later, clearAll} = useTimeouts()

    watch(() => toValue(source), (on) => {
        clearAll()
        if (on) {
            held.value = true
            return
        }
        later(() => (held.value = false), holdMs)
    })

    return readonly(held)
}
