import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'

function touch(type, x, y) {
    const event = new Event(type)
    event.touches = type === 'touchend' ? [] : [{clientX: x, clientY: y}]
    window.dispatchEvent(event)
}

const range = {target: {type: 'range', value: '5'}}

describe('useScrollProtect', () => {
    let blockIfScrolling, isScrolling

    beforeEach(async () => {
        vi.useFakeTimers()
        // Its state and window listeners are module-wide: a fresh module per test.
        vi.resetModules()
        const {useScrollProtect} = await import('./useScrollProtect.js')
        ;({blockIfScrolling, isScrolling} = useScrollProtect())
    })

    afterEach(() => {
        touch('touchcancel', 0, 0)
        vi.useRealTimers()
    })

    it('blocks a control while the finger scrolls, and briefly after it lifts', () => {
        touch('touchstart', 100, 100)
        touch('touchmove', 102, 140)
        expect(isScrolling.value).toBe(true)
        expect(blockIfScrolling(range)).toBe(true)

        touch('touchend')
        expect(blockIfScrolling(range)).toBe(true)
        vi.advanceTimersByTime(50)
        expect(blockIfScrolling(range)).toBe(false)
    })

    it('holds a slider still until the gesture shows it is a slide', () => {
        touch('touchstart', 100, 100)
        expect(blockIfScrolling(range)).toBe(true)

        touch('touchmove', 140, 102)
        expect(blockIfScrolling(range)).toBe(false)
    })

    it('keeps the next gesture\'s intent when it starts inside the release delay', () => {
        touch('touchstart', 100, 100)
        touch('touchmove', 102, 140)
        touch('touchend')

        touch('touchstart', 100, 100)
        touch('touchmove', 140, 102)
        vi.advanceTimersByTime(50)

        // Still mid-slide: the old scroll's reset must not reopen the 5 px dead zone.
        expect(blockIfScrolling(range)).toBe(false)
        touch('touchmove', 150, 160)
        expect(isScrolling.value).toBe(false)
    })
})
