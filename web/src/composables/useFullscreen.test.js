import {describe, it, expect, vi, beforeEach} from 'vitest'
import {useFullscreen} from './useFullscreen.js'

function setFullscreenElement(element) {
    Object.defineProperty(document, 'fullscreenElement', {
        value: element,
        writable: true,
        configurable: true,
    })
}

describe('useFullscreen', () => {
    beforeEach(() => {
        setFullscreenElement(null)
        Element.prototype.requestFullscreen = vi.fn().mockResolvedValue(undefined)
        document.exitFullscreen = vi.fn().mockResolvedValue(undefined)
    })

    it('starts out of fullscreen', () => {
        const {isFullscreen} = useFullscreen()

        expect(isFullscreen.value).toBe(false)
    })

    it('requests fullscreen on the element it is given', () => {
        const {toggleFullscreen} = useFullscreen()
        const element = document.createElement('div')

        toggleFullscreen(element)

        expect(Element.prototype.requestFullscreen).toHaveBeenCalled()
    })

    it('exits when something is already fullscreen', () => {
        setFullscreenElement(document.body)
        const {toggleFullscreen} = useFullscreen()

        toggleFullscreen(document.createElement('div'))

        expect(document.exitFullscreen).toHaveBeenCalled()
        expect(Element.prototype.requestFullscreen).not.toHaveBeenCalled()
    })

    // The request is not the state: it can be refused, and the browser leaves
    // fullscreen on its own (Esc). Only the event may move `isFullscreen`.
    it('does not claim fullscreen until the browser confirms', () => {
        const {isFullscreen, toggleFullscreen, handleFullscreenChange} = useFullscreen()

        toggleFullscreen(document.createElement('div'))
        expect(isFullscreen.value).toBe(false)

        setFullscreenElement(document.body)
        handleFullscreenChange()
        expect(isFullscreen.value).toBe(true)
    })

    it('follows the browser back out of fullscreen', () => {
        const {isFullscreen, handleFullscreenChange} = useFullscreen()
        setFullscreenElement(document.body)
        handleFullscreenChange()

        setFullscreenElement(null)
        handleFullscreenChange()

        expect(isFullscreen.value).toBe(false)
    })

    describe('onChange', () => {
        it('fires when fullscreen is entered', () => {
            const onChange = vi.fn()
            const {handleFullscreenChange} = useFullscreen({onChange})

            setFullscreenElement(document.body)
            handleFullscreenChange()

            expect(onChange).toHaveBeenCalledTimes(1)
            expect(onChange).toHaveBeenCalledWith(true)
        })

        // Only the edges: a resize inside fullscreen fires `fullscreenchange` on
        // some browsers, and re-fitting on each one would fight the user's zoom.
        it('does not fire again while already fullscreen', () => {
            const onChange = vi.fn()
            const {handleFullscreenChange} = useFullscreen({onChange})
            setFullscreenElement(document.body)
            handleFullscreenChange()

            handleFullscreenChange()

            expect(onChange).toHaveBeenCalledTimes(1)
        })

        it('fires on the way out', () => {
            const onChange = vi.fn()
            const {handleFullscreenChange} = useFullscreen({onChange})
            setFullscreenElement(document.body)
            handleFullscreenChange()
            onChange.mockClear()

            setFullscreenElement(null)
            handleFullscreenChange()

            expect(onChange).toHaveBeenCalledTimes(1)
            expect(onChange).toHaveBeenCalledWith(false)
        })

        it('does not fire while never fullscreen', () => {
            const onChange = vi.fn()
            const {handleFullscreenChange} = useFullscreen({onChange})

            setFullscreenElement(null)
            handleFullscreenChange()

            expect(onChange).not.toHaveBeenCalled()
        })

        it('fires on every edge across two entries', () => {
            const onChange = vi.fn()
            const {handleFullscreenChange} = useFullscreen({onChange})

            setFullscreenElement(document.body)
            handleFullscreenChange()
            setFullscreenElement(null)
            handleFullscreenChange()
            setFullscreenElement(document.body)
            handleFullscreenChange()

            expect(onChange.mock.calls.map(([entered]) => entered)).toEqual([true, false, true])
        })
    })
})
