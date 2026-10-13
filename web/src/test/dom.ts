import {vi} from 'vitest'
import type {VueWrapper} from '@vue/test-utils'

/**
 * A `ResizeObserver` fired by hand: happy-dom does no layout, so it never reports a
 * reshape on its own. Vitest's `unstubGlobals`
 * restores the real one after each test.
 */
export function installResizeObserverStub() {
    const callbacks = new Set<ResizeObserverCallback>()
    vi.stubGlobal('ResizeObserver', class {
        callback: ResizeObserverCallback

        constructor(callback: ResizeObserverCallback) {
            this.callback = callback
        }

        observe() {
            callbacks.add(this.callback)
        }

        unobserve() {}

        disconnect() {
            callbacks.delete(this.callback)
        }
    })
    return {
        /** Lays the mounted view out at `width`x`height` and reports it. */
        resizeContainer(wrapper: VueWrapper, width: number, height: number) {
            const element = wrapper.element as Element
            element.getBoundingClientRect = () =>
                DOMRect.fromRect({x: 0, y: 0, width, height})
            callbacks.forEach((callback) => callback([], {} as ResizeObserver))
        },
    }
}
