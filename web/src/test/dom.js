import {vi} from 'vitest'

/**
 * A `ResizeObserver` fired by hand: happy-dom does no layout, so it never reports a
 * reshape on its own. Vitest's `unstubGlobals`
 * restores the real one after each test.
 */
export function installResizeObserverStub() {
    const callbacks = new Set()
    vi.stubGlobal('ResizeObserver', class {
        constructor(callback) {
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
        resizeContainer(wrapper, width, height) {
            wrapper.element.getBoundingClientRect = () =>
                ({left: 0, top: 0, x: 0, y: 0, width, height, right: width, bottom: height})
            callbacks.forEach((callback) => callback([]))
        },
    }
}
