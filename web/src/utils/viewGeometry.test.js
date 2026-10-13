import {clamp, fitScale, offsetBounds, touchCenter, touchDistance} from './viewGeometry.js'

const touch = (clientX, clientY) => ({clientX, clientY})

describe('viewGeometry', () => {
    it('clamps into a range', () => {
        expect(clamp(5, 0, 2)).toBe(2)
        expect(clamp(-1, 0, 2)).toBe(0)
        expect(clamp(1.5, 0, 2)).toBe(1.5)
    })

    it('fits by the tighter side and never enlarges', () => {
        expect(fitScale({width: 800, height: 600}, 1600, 600)).toBe(0.5)
        expect(fitScale({width: 800, height: 600}, 400, 300)).toBe(1)
        expect(fitScale({width: 800, height: 600}, 1600, 1200, 0.95)).toBeCloseTo(0.475)
    })

    it('measures and centres a pinch', () => {
        const touches = [touch(0, 0), touch(30, 40)]
        expect(touchDistance(touches)).toBe(50)
        expect(touchCenter(touches)).toEqual({x: 15, y: 20})
        expect(touchCenter([touch(7, 9)])).toEqual({x: 7, y: 9})
    })

    it('reads an element\'s offset box', () => {
        const element = {offsetLeft: 10, offsetTop: 175, offsetWidth: 800, offsetHeight: 450}
        expect(offsetBounds(element)).toEqual({left: 10, top: 175, width: 800, height: 450})
    })
})
