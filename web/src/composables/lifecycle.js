import {getCurrentInstance, getCurrentScope, onBeforeUnmount, onMounted, onScopeDispose} from 'vue'

/*
 * The lifecycle hooks a composable registers, safe to call from anywhere: inside a
 * component (or, for the dispose hook, an effect scope) they are Vue's own; outside one —
 * a test calling the composable bare, a store built at module level — they do nothing,
 * where Vue's would only warn. Every hook in `composables/` goes through these.
 */

/** Runs `fn` when the calling component or effect scope goes away. */
export function tryOnScopeDispose(fn) {
    if (getCurrentScope()) onScopeDispose(fn)
}

/** Runs `fn` once the calling component is mounted. */
export function tryOnMounted(fn) {
    if (getCurrentInstance()) onMounted(fn)
}

/** Runs `fn` before the calling component unmounts, while it can still emit. */
export function tryOnBeforeUnmount(fn) {
    if (getCurrentInstance()) onBeforeUnmount(fn)
}
