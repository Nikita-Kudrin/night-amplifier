import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {effectScope} from 'vue'
import {useTimeouts} from './useTimeouts'

describe('useTimeouts', () => {
    beforeEach(() => vi.useFakeTimers())
    afterEach(() => vi.useRealTimers())

    function inScope() {
        const scope = effectScope()
        const timeouts = scope.run(() => useTimeouts())!
        return {scope, ...timeouts}
    }

    it('runs a callback after its delay', () => {
        const {later} = inScope()
        const callback = vi.fn()

        later(callback, 500)
        vi.advanceTimersByTime(499)
        expect(callback).not.toHaveBeenCalled()
        vi.advanceTimersByTime(1)
        expect(callback).toHaveBeenCalledTimes(1)
    })

    it('drops every pending callback when its owner goes away', () => {
        const {scope, later} = inScope()
        const callbacks = [vi.fn(), vi.fn()]

        later(callbacks[0], 10)
        later(callbacks[1], 500)
        scope.stop()
        vi.advanceTimersByTime(1000)

        callbacks.forEach((callback) => expect(callback).not.toHaveBeenCalled())
        expect(vi.getTimerCount()).toBe(0)
    })

    it('forgets a timer once it fired', () => {
        const {later, clearAll} = inScope()
        const id = later(() => {}, 10)
        vi.advanceTimersByTime(10)
        const clear = vi.spyOn(globalThis, 'clearTimeout')

        clearAll()

        expect(clear).not.toHaveBeenCalledWith(id)
    })
})
