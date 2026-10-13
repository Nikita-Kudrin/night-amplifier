import {effectScope} from 'vue'
import {useEventListener} from './useEventListener.js'

describe('useEventListener', () => {
    it('listens until its scope goes away', () => {
        const target = new window.EventTarget()
        const listener = vi.fn()
        const scope = effectScope()
        scope.run(() => useEventListener(target, 'resize', listener))

        target.dispatchEvent(new Event('resize'))
        scope.stop()
        target.dispatchEvent(new Event('resize'))

        expect(listener).toHaveBeenCalledTimes(1)
    })

    it('can be removed early', () => {
        const target = new window.EventTarget()
        const listener = vi.fn()
        const stop = useEventListener(target, 'change', listener)

        stop()
        target.dispatchEvent(new Event('change'))

        expect(listener).not.toHaveBeenCalled()
    })

    // `screen.orientation` is missing before Safari 16.4.
    it('ignores a missing target', () => {
        expect(() => useEventListener(undefined, 'change', vi.fn())()).not.toThrow()
    })
})
