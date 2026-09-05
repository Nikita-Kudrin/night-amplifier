import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {IDLE_HIDE_MS, useOverlayVisibility} from './useOverlayVisibility.js'

/** A press event on the image itself, at an optional position. */
function imagePress(x = 0, y = 0) {
    return {clientX: x, clientY: y, target: {closest: () => null}}
}

/** A press event that landed on a control. */
function controlPress(x = 0, y = 0) {
    return {clientX: x, clientY: y, target: {closest: () => ({})}}
}

function touch(x, y, count = 1) {
    const points = Array.from({length: count}, () => ({clientX: x, clientY: y}))
    return {touches: points, changedTouches: [{clientX: x, clientY: y}], target: {closest: () => null}}
}

describe('useOverlayVisibility', () => {
    beforeEach(() => {
        vi.useFakeTimers()
    })

    afterEach(() => {
        vi.useRealTimers()
    })

    it('starts visible', () => {
        const {visible} = useOverlayVisibility()

        expect(visible.value).toBe(true)
    })

    // Opening the view and not touching it *is* inactivity. Without a countdown
    // armed at creation the controls sat on the image for the whole session.
    it('fades on its own when nothing ever happens', () => {
        const {visible} = useOverlayVisibility()

        vi.advanceTimersByTime(IDLE_HIDE_MS)

        expect(visible.value).toBe(false)
    })

    it('hides after the idle timeout once shown', () => {
        const {visible, show} = useOverlayVisibility()

        show()
        expect(visible.value).toBe(true)

        vi.advanceTimersByTime(IDLE_HIDE_MS - 1)
        expect(visible.value).toBe(true)

        vi.advanceTimersByTime(1)
        expect(visible.value).toBe(false)
    })

    it('restarts the countdown on each show, rather than stacking timers', () => {
        const {visible, show} = useOverlayVisibility()

        show()
        vi.advanceTimersByTime(IDLE_HIDE_MS - 100)
        show()
        vi.advanceTimersByTime(IDLE_HIDE_MS - 100)

        expect(visible.value).toBe(true)

        vi.advanceTimersByTime(100)
        expect(visible.value).toBe(false)
    })

    it('honours a custom idle timeout', () => {
        const {visible, show} = useOverlayVisibility({idleMs: 50})

        show()
        vi.advanceTimersByTime(50)

        expect(visible.value).toBe(false)
    })

    describe('tap on the image', () => {
        it('shows the controls when they are hidden', () => {
            const {visible, show, handlePressStart, handlePressEnd} = useOverlayVisibility()
            show()
            vi.advanceTimersByTime(IDLE_HIDE_MS)
            expect(visible.value).toBe(false)

            handlePressStart(imagePress(10, 10))
            handlePressEnd(imagePress(10, 10))

            expect(visible.value).toBe(true)
        })

        it('hides them again on a second tap', () => {
            const {visible, handlePressStart, handlePressEnd} = useOverlayVisibility()
            expect(visible.value).toBe(true)

            handlePressStart(imagePress(10, 10))
            handlePressEnd(imagePress(10, 10))

            expect(visible.value).toBe(false)
        })

        // The press itself may have made them visible, so a toggle reading the
        // *current* visibility on release would immediately undo the tap that
        // opened them.
        it('decides from the visibility at press time, not release', () => {
            const {visible, show, handlePressStart, handlePressEnd} = useOverlayVisibility()
            show()
            vi.advanceTimersByTime(IDLE_HIDE_MS)

            handlePressStart(imagePress())
            show() // something else woke the overlay mid-press
            handlePressEnd(imagePress())

            expect(visible.value).toBe(true)
        })
    })

    describe('gestures and controls', () => {
        it('treats a press that travelled as a drag, not a tap', () => {
            const {visible, handlePressStart, handlePressEnd} = useOverlayVisibility()

            handlePressStart(imagePress(0, 0))
            handlePressEnd(imagePress(40, 40))

            expect(visible.value).toBe(true)
        })

        it('keeps the controls up when the press landed on one', () => {
            const {visible, handlePressStart, handlePressEnd} = useOverlayVisibility()

            handlePressStart(controlPress(5, 5))
            handlePressEnd(controlPress(5, 5))

            expect(visible.value).toBe(true)
        })

        it('shows the controls when a hidden one is pressed through', () => {
            const {visible, show, handlePressStart} = useOverlayVisibility()
            show()
            vi.advanceTimersByTime(IDLE_HIDE_MS)

            handlePressStart(controlPress())

            expect(visible.value).toBe(true)
        })

        it('does not toggle at the end of a pinch', () => {
            const {visible, handlePressStart, handlePressEnd} = useOverlayVisibility()

            handlePressStart(touch(10, 10, 2))
            handlePressEnd(touch(10, 10, 2))

            expect(visible.value).toBe(true)
        })

        it('toggles on a single-finger tap', () => {
            const {visible, handlePressStart, handlePressEnd} = useOverlayVisibility()

            handlePressStart(touch(10, 10))
            handlePressEnd(touch(10, 10))

            expect(visible.value).toBe(false)
        })

        // `touches` is empty on touchend — only `changedTouches` carries the finger
        // that lifted, so reading the wrong one would measure travel from (0, 0)
        // and call every tap a drag.
        it('measures touch travel from changedTouches', () => {
            const {visible, handlePressStart, handlePressEnd} = useOverlayVisibility()

            handlePressStart(touch(300, 300))
            handlePressEnd({
                touches: [],
                changedTouches: [{clientX: 301, clientY: 300}],
                target: {closest: () => null},
            })

            expect(visible.value).toBe(false)
        })
    })

    // A dropdown is teleported out of the controls, so it neither fades with them
    // nor feeds them pointer events. Left to the countdown, its button would fade
    // out from under an open menu.
    describe('setHold', () => {
        it('keeps the controls up for as long as the hold lasts', () => {
            const {visible, show, setHold} = useOverlayVisibility()
            show()

            setHold(true)
            vi.advanceTimersByTime(IDLE_HIDE_MS * 2)

            expect(visible.value).toBe(true)
        })

        it('shows the controls even if they had already gone', () => {
            const {visible, show, setHold} = useOverlayVisibility()
            show()
            vi.advanceTimersByTime(IDLE_HIDE_MS)
            expect(visible.value).toBe(false)

            setHold(true)

            expect(visible.value).toBe(true)
        })

        it('restarts the countdown when released', () => {
            const {visible, setHold} = useOverlayVisibility()
            setHold(true)

            setHold(false)
            expect(visible.value).toBe(true)

            vi.advanceTimersByTime(IDLE_HIDE_MS)
            expect(visible.value).toBe(false)
        })
    })

    /**
     * A browser replays a touch as compatibility mouse events unless the touch
     * handler calls `preventDefault()` — which the live view cannot, its touch
     * listeners being passive. Both views bind touch *and* mouse, so one tap
     * arrived as two presses and the toggle undid itself: the controls never moved
     * on a phone, the one device the fade exists for.
     */
    describe('compatibility mouse events after a touch', () => {
        const touchTap = (x = 10, y = 10) => [
            {
                touches: [{clientX: x, clientY: y}],
                changedTouches: [{clientX: x, clientY: y}],
                target: {closest: () => null},
            },
            {touches: [], changedTouches: [{clientX: x, clientY: y}], target: {closest: () => null}},
        ]

        it('counts one tap as one toggle', () => {
            const {visible, handlePressStart, handlePressEnd} = useOverlayVisibility()
            const [start, end] = touchTap()

            handlePressStart(start)
            handlePressEnd(end)
            expect(visible.value).toBe(false)

            // What the browser synthesises immediately afterwards.
            handlePressStart(imagePress(10, 10))
            handlePressEnd(imagePress(10, 10))

            expect(visible.value).toBe(false)
        })

        it('lets a real mouse through once the ghost window has passed', () => {
            const {visible, handlePressStart, handlePressEnd} = useOverlayVisibility()
            const [start, end] = touchTap()
            handlePressStart(start)
            handlePressEnd(end)

            vi.advanceTimersByTime(800)
            handlePressStart(imagePress(10, 10))
            handlePressEnd(imagePress(10, 10))

            expect(visible.value).toBe(true)
        })

        it('does not disown a mouse press on a mouse-only device', () => {
            const {visible, handlePressStart, handlePressEnd} = useOverlayVisibility()

            handlePressStart(imagePress(10, 10))
            handlePressEnd(imagePress(10, 10))

            expect(visible.value).toBe(false)
        })
    })

    /**
     * `touchcancel` is the system taking the gesture away — a notification swipe,
     * palm rejection — not the observer asking for anything.
     */
    describe('cancelPress', () => {
        it('does not toggle', () => {
            const {visible, handlePressStart, cancelPress} = useOverlayVisibility()

            handlePressStart(imagePress(10, 10))
            cancelPress(imagePress(10, 10))

            expect(visible.value).toBe(true)
        })

        it('leaves no press for a stray release to finish', () => {
            const {visible, handlePressStart, handlePressEnd, cancelPress} = useOverlayVisibility()

            handlePressStart(imagePress(10, 10))
            cancelPress(imagePress(10, 10))
            handlePressEnd(imagePress(10, 10))

            expect(visible.value).toBe(true)
        })
    })

    it('hide clears a pending countdown', () => {
        const {visible, show, hide} = useOverlayVisibility()

        show()
        hide()
        vi.advanceTimersByTime(IDLE_HIDE_MS * 2)

        expect(visible.value).toBe(false)
    })
})
