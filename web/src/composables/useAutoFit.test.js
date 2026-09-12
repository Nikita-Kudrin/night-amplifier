import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {useAutoFit, FIT_SETTLE_MS} from './useAutoFit.js'

describe('useAutoFit', () => {
    let fit

    beforeEach(() => {
        vi.useFakeTimers()
        fit = vi.fn()
    })

    afterEach(() => {
        vi.useRealTimers()
    })

    function laidOutAt(width, height) {
        const autoFit = useAutoFit({fit})
        autoFit.handleContainerResize(width, height)
        return autoFit
    }

    // Leaving fullscreen: the event lands while the container still has the
    // fullscreen size, so fitting then would fit to the viewport being left.
    it('does not fit when asked, only once the container reshapes', () => {
        const {requestFit, handleContainerResize} = laidOutAt(1920, 1080)

        requestFit()
        expect(fit).not.toHaveBeenCalled()

        handleContainerResize(800, 450)
        expect(fit).toHaveBeenCalledTimes(1)
    })

    // Android slides its URL bar back in after the first resize: the second
    // reshape arrives past the original window and must still be followed.
    it('follows every reshape while the layout is still settling', () => {
        const {requestFit, handleContainerResize} = laidOutAt(1920, 1080)
        requestFit()

        handleContainerResize(1080, 1920)
        vi.advanceTimersByTime(FIT_SETTLE_MS - 1)
        handleContainerResize(1080, 1800)

        expect(fit).toHaveBeenCalledTimes(2)
    })

    it('falls back to one fit when the container never reshapes', () => {
        const {requestFit} = laidOutAt(1920, 1080)
        requestFit()

        vi.advanceTimersByTime(FIT_SETTLE_MS - 1)
        expect(fit).not.toHaveBeenCalled()

        vi.advanceTimersByTime(1)
        expect(fit).toHaveBeenCalledTimes(1)
    })

    // A late extra fit would undo a pinch made right after rotating.
    it('does not fit again when the window closes after a reshape fitted', () => {
        const {requestFit, handleContainerResize} = laidOutAt(1920, 1080)
        requestFit()
        handleContainerResize(800, 450)

        vi.advanceTimersByTime(FIT_SETTLE_MS * 2)

        expect(fit).toHaveBeenCalledTimes(1)
    })

    // Dragging the sidebar or resizing a desktop window keeps the user's zoom.
    it('ignores reshapes nobody asked a fit for', () => {
        const {handleContainerResize} = laidOutAt(1920, 1080)

        handleContainerResize(800, 450)
        vi.advanceTimersByTime(FIT_SETTLE_MS * 2)

        expect(fit).not.toHaveBeenCalled()
    })

    it('stops following reshapes once settled', () => {
        const {requestFit, handleContainerResize} = laidOutAt(1920, 1080)
        requestFit()
        vi.advanceTimersByTime(FIT_SETTLE_MS)
        fit.mockClear()

        handleContainerResize(800, 450)

        expect(fit).not.toHaveBeenCalled()
    })

    // Zooming and every new frame re-report the container at the same size.
    it('does not count a same-size report as a reshape', () => {
        const {requestFit, handleContainerResize} = laidOutAt(1920, 1080)
        requestFit()

        handleContainerResize(1920, 1080)
        expect(fit).not.toHaveBeenCalled()

        vi.advanceTimersByTime(FIT_SETTLE_MS)
        expect(fit).toHaveBeenCalledTimes(1)
    })

    it('needs a first size before a report counts as a reshape', () => {
        const {requestFit, handleContainerResize} = useAutoFit({fit})
        requestFit()

        handleContainerResize(800, 450)
        expect(fit).not.toHaveBeenCalled()

        vi.advanceTimersByTime(FIT_SETTLE_MS)
        expect(fit).toHaveBeenCalledTimes(1)
    })

    // One rotation can fire both `orientationchange` and `screen.orientation`'s
    // `change`.
    it('fits once when asked twice for the same change', () => {
        const {requestFit} = laidOutAt(1920, 1080)

        requestFit()
        vi.advanceTimersByTime(100)
        requestFit()
        vi.advanceTimersByTime(FIT_SETTLE_MS * 2)

        expect(fit).toHaveBeenCalledTimes(1)
    })

    it('does not fall back when asked again after a reshape already fitted', () => {
        const {requestFit, handleContainerResize} = laidOutAt(1920, 1080)
        requestFit()
        handleContainerResize(1080, 1920)

        requestFit()
        vi.advanceTimersByTime(FIT_SETTLE_MS * 2)

        expect(fit).toHaveBeenCalledTimes(1)
    })

    it('arms afresh once the previous window has closed', () => {
        const {requestFit} = laidOutAt(1920, 1080)
        requestFit()
        vi.advanceTimersByTime(FIT_SETTLE_MS)

        requestFit()
        vi.advanceTimersByTime(FIT_SETTLE_MS)

        expect(fit).toHaveBeenCalledTimes(2)
    })

    it('does not fit after dispose', () => {
        const {requestFit, handleContainerResize, dispose} = laidOutAt(1920, 1080)
        requestFit()

        dispose()
        handleContainerResize(800, 450)
        vi.advanceTimersByTime(FIT_SETTLE_MS * 2)

        expect(fit).not.toHaveBeenCalled()
    })
})
