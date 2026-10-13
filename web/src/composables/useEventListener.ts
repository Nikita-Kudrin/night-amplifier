import {tryOnScopeDispose} from './lifecycle.js'

/**
 * `target.addEventListener(type, listener, options)`, removed when the calling component
 * (or effect scope) goes away, so a window or document listener cannot outlive its view.
 * A missing target (`screen.orientation` before Safari 16.4) is a no-op. Returns a function
 * that removes the listener early.
 */
export function useEventListener<K extends keyof WindowEventMap>(
    target: Window | null | undefined,
    type: K,
    listener: (event: WindowEventMap[K]) => void,
    options?: boolean | AddEventListenerOptions,
): () => void
export function useEventListener<K extends keyof DocumentEventMap>(
    target: Document | null | undefined,
    type: K,
    listener: (event: DocumentEventMap[K]) => void,
    options?: boolean | AddEventListenerOptions,
): () => void
export function useEventListener(
    target: EventTarget | null | undefined,
    type: string,
    listener: (event: Event) => void,
    options?: boolean | AddEventListenerOptions,
): () => void
export function useEventListener(
    target: EventTarget | null | undefined,
    type: string,
    listener: (event: Event) => void,
    options?: boolean | AddEventListenerOptions,
): () => void {
    if (!target) return () => {}
    target.addEventListener(type, listener, options)
    const stop = () => target.removeEventListener(type, listener, options)
    tryOnScopeDispose(stop)
    return stop
}
