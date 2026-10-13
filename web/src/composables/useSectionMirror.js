import {reactive, watch} from 'vue'
import {copySetting} from '../utils/settings.js'

/**
 * A settings section's editable copy of the keys it owns, following `source()` (the
 * panel's mirror) deeply: a kept-mounted panel follows server state, and two sections
 * edit the `eyepiece` group, so a stale copy would undo the other's edit. Groups are
 * copied, so an edit stays local until the section emits `apply(key, value)`.
 */
export function useSectionMirror(source, keys) {
    const local = reactive({})
    watch(
        () => keys.map((key) => source()[key]),
        (values) => keys.forEach((key, i) => (local[key] = copySetting(values[i]))),
        {immediate: true, deep: true}
    )
    return local
}
