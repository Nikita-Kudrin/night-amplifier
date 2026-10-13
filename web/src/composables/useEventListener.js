import {tryOnScopeDispose} from './lifecycle.js'

/**
 * `target.addEventListener(type, listener, options)`, removed when the calling component
 * (or effect scope) goes away, so a window or document listener cannot outlive its view.
 * A missing target (`screen.orientation` before Safari 16.4) is a no-op.
 * @returns {() => void} Removes the listener early.
 */
export function useEventListener(target, type, listener, options) {
    if (!target) return () => {}
    target.addEventListener(type, listener, options)
    const stop = () => target.removeEventListener(type, listener, options)
    tryOnScopeDispose(stop)
    return stop
}
