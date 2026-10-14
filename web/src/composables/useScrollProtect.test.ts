import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {__resetForTests, createScrollProtect, useScrollProtect} from './useScrollProtect'
import {TOUCH_SLOP_PX} from '../constants'

type ScrollProtect = ReturnType<typeof createScrollProtect>

/** A touch event as the guard reads it: jsdom has no `Touch` to build a real one from. */
function touchEvent(type: string, touches: {clientX: number; clientY: number}[]) {
    return Object.assign(new Event(type), {touches})
}

function touch(type: string, x = 0, y = 0) {
    window.dispatchEvent(touchEvent(type, type === 'touchend' ? [] : [{clientX: x, clientY: y}]))
}

const slider = document.createElement('input')
slider.type = 'range'
slider.value = '5'
const range = {target: slider}

describe('useScrollProtect', () => {
    let blockIfScrolling: ScrollProtect['blockIfScrolling']
    let isScrolling: ScrollProtect['isScrolling']

    beforeEach(() => {
        vi.useFakeTimers()
        ;({blockIfScrolling, isScrolling} = useScrollProtect())
    })

    afterEach(() => {
        __resetForTests()
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

        // Still mid-slide: the old scroll's reset must not reopen the dead zone.
        expect(blockIfScrolling(range)).toBe(false)
        touch('touchmove', 150, 160)
        expect(isScrolling.value).toBe(false)
    })

    it('decides nothing inside the touch slop', () => {
        touch('touchstart', 100, 100)
        touch('touchmove', 100, 100 + TOUCH_SLOP_PX)
        expect(isScrolling.value).toBe(false)
        touch('touchmove', 100, 101 + TOUCH_SLOP_PX)
        expect(isScrolling.value).toBe(true)
    })

    it('is one guard for the page, and lets go of the window once reset', () => {
        expect(useScrollProtect().blockIfScrolling).toBe(blockIfScrolling)
        __resetForTests()

        touch('touchstart', 100, 100)
        touch('touchmove', 102, 140)
        expect(isScrolling.value).toBe(false)
        expect(useScrollProtect().isScrolling).not.toBe(isScrolling)
    })

    it('watches the target it is given', () => {
        const target = document.createElement('div')
        const guard = createScrollProtect(target)
        const send = (type: string, y: number) => target.dispatchEvent(touchEvent(type, [{clientX: 100, clientY: y}]))

        send('touchstart', 100)
        send('touchmove', 140)
        expect(guard.isScrolling.value).toBe(true)
        guard.dispose()
    })
})
