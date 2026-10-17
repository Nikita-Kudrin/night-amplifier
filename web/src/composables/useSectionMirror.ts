import {reactive, watch} from 'vue'
import {copySetting} from '../utils/settings'

/**
 * A settings section's editable copy of the keys it owns, following `source()` (the
 * panel's mirror) deeply, as a kept-mounted panel follows server state. Groups are
 * copied, so an edit stays local until the section emits `apply(key, value)`.
 *
 * `source()` is the panel's `Settings` mirror, or one of its groups (`eyepiece`), whose
 * keys `keys` must name.
 *
 * A sync leaves the keys in `held` (edited, not yet sent) alone: a screen size waiting out
 * its debounce was overwritten by another section's save landing, then sent as the old one.
 */
export function useSectionMirror<T extends object, K extends keyof T>(
    source: () => T,
    keys: readonly K[],
    held: ReadonlySet<K> = new Set(),
): Pick<T, K> {
    // Filled before this returns: the watcher runs at once.
    const local = reactive({}) as Pick<T, K>
    watch(
        () => keys.map((key) => source()[key]),
        (values) => keys.forEach((key, i) => {
            if (!held.has(key)) local[key] = copySetting(values[i])
        }),
        {immediate: true, deep: true}
    )
    return local
}
