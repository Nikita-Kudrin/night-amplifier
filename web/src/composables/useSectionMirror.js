import {reactive, watch} from 'vue'
import {copySetting} from '../utils/settings.js'

/**
 * A settings section's editable copy of the keys it owns, following `source()` (the
 * panel's mirror) deeply, as a kept-mounted panel follows server state. Groups are
 * copied, so an edit stays local until the section emits `apply(key, value)`.
 *
 * A sync leaves the keys in `held` (edited, not yet sent) alone: a screen size waiting out
 * its debounce was overwritten by another section's save landing, then sent as the old one.
 */
export function useSectionMirror(source, keys, held = new Set()) {
    const local = reactive({})
    watch(
        () => keys.map((key) => source()[key]),
        (values) => keys.forEach((key, i) => {
            if (!held.has(key)) local[key] = copySetting(values[i])
        }),
        {immediate: true, deep: true}
    )
    return local
}
