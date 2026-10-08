import {describe, it, expect, beforeEach} from 'vitest'
import {useEyepieceZoom} from '../useEyepieceZoom.js'

const touch = (x, y) => ({clientX: x, clientY: y})

describe('useEyepieceZoom', () => {
    beforeEach(() => {
        window.innerWidth = 1000
        window.innerHeight = 800
    })

    it('zooms about the pointer, never past 2x', () => {
        const zoom = useEyepieceZoom(() => true)

        zoom.wheel({deltaY: -500, clientX: 0, clientY: 0})
        expect(zoom.scale.value).toBe(1.5)
        expect(zoom.style.value.transform).toBe('translate(0px, 0px) scale(1.5)')

        zoom.wheel({deltaY: -5000, clientX: 0, clientY: 0})
        expect(zoom.scale.value).toBe(2)
    })

    it('never pans an edge of the image inside the screen', () => {
        const zoom = useEyepieceZoom(() => true)
        zoom.wheel({deltaY: -1000, clientX: 1000, clientY: 800})

        zoom.touchStart({touches: [touch(500, 400)]})
        zoom.touchMove({touches: [touch(5000, 4000)]})
        expect(zoom.style.value.transform).toBe('translate(0px, 0px) scale(2)')

        zoom.touchMove({touches: [touch(-5000, -4000)]})
        expect(zoom.style.value.transform).toBe('translate(-1000px, -800px) scale(2)')
    })

    it('pinches between 1x and 2x', () => {
        const zoom = useEyepieceZoom(() => true)
        zoom.touchStart({touches: [touch(400, 400), touch(600, 400)]})

        zoom.touchMove({touches: [touch(350, 400), touch(650, 400)]})
        expect(zoom.scale.value).toBe(1.5)

        zoom.touchMove({touches: [touch(100, 400), touch(900, 400)]})
        expect(zoom.scale.value).toBe(2)

        zoom.touchMove({touches: [touch(490, 400), touch(510, 400)]})
        expect(zoom.scale.value).toBe(1)
        expect(zoom.style.value).toEqual({})
    })

    it('does nothing while disabled, and reset fits the view', () => {
        let enabled = false
        const zoom = useEyepieceZoom(() => enabled)
        zoom.wheel({deltaY: -1000, clientX: 0, clientY: 0})
        expect(zoom.scale.value).toBe(1)

        enabled = true
        zoom.wheel({deltaY: -1000, clientX: 0, clientY: 0})
        zoom.reset()
        expect(zoom.scale.value).toBe(1)
        expect(zoom.style.value).toEqual({})
    })
})
