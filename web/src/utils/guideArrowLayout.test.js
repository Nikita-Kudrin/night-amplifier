import {
    ARROW_ASPECT,
    arrowScale,
    baseSize,
    edgeOffset,
    formatScreens,
    infoOffset,
    isOffScreen,
    screensAway,
} from './guideArrowLayout.js'

describe('guideArrowLayout', () => {
    it('counts screens only once the FOV is known', () => {
        expect(screensAway(2, 0)).toBeNull()
        expect(screensAway(2, 1)).toBe(2)
    })

    it('moves the arrow to the edge past 0.4 of a screen', () => {
        expect(isOffScreen(null)).toBe(false)
        expect(isOffScreen(0.4)).toBe(false)
        expect(isOffScreen(0.41)).toBe(true)
    })

    it('grows with edge proximity, quadratically, between 0.3 and 1.6', () => {
        expect(arrowScale(0, 0)).toBeCloseTo(0.3)
        expect(arrowScale(0, 0.2)).toBeCloseTo(0.3 + 1.3 * 0.25)
        expect(arrowScale(0, 5)).toBeCloseTo(1.6)
    })

    it('falls back to the distance before a plate solve', () => {
        expect(arrowScale(15, null)).toBeCloseTo(0.3 + 1.3 * 0.5)
        expect(arrowScale(90, null)).toBeCloseTo(1.6)
    })

    it('sizes with the smaller side, within its bounds', () => {
        expect(baseSize(600, 1200, false)).toBe(100)
        expect(baseSize(100, 100, false)).toBe(60)
        expect(baseSize(6000, 6000, false)).toBe(240)
        expect(baseSize(6000, 6000, true)).toBe(290)
    })

    it.each([
        [0, {x: 0, y: -1}],
        [90, {x: 1, y: 0}],
        [180, {x: 0, y: 1}],
        [270, {x: -1, y: 0}],
    ])('points toward %s° and stops a margin and half an arrow short of the edge', (angleDeg, dir) => {
        const arrowSize = 50
        const {x, y} = edgeOffset({angleDeg, arrowSize, imageWidth: 800, imageHeight: 600})
        // Pointing along an axis the rotated arrow's half-extent along it is half its height.
        const halfAlong = arrowSize * ARROW_ASPECT * 0.5
        if (dir.x) expect(x).toBeCloseTo(dir.x * (400 - halfAlong - 80))
        if (dir.y) expect(y).toBeCloseTo(dir.y * (300 - halfAlong - 60))
        expect(Math.abs(dir.x ? y : x)).toBeLessThan(1e-9)
    })

    it('stays on the circle of a circular eyepiece', () => {
        const {x, y} = edgeOffset({angleDeg: 45, arrowSize: 50, imageWidth: 800, imageHeight: 600, isCircular: true})
        expect(Math.hypot(x, y)).toBeCloseTo(300 - 40 - 30)
        expect(x).toBeCloseTo(-y)
    })

    it('pins to the centre when the arrow fills the image', () => {
        const {x, y} = edgeOffset({angleDeg: 90, arrowSize: 500, imageWidth: 400, imageHeight: 300})
        expect(Math.hypot(x, y)).toBe(0)
    })

    it('puts the readout on the tail side', () => {
        const up = infoOffset(0, 100)
        expect(up.x).toBeCloseTo(0)
        expect(up.y).toBeCloseTo(100 * ARROW_ASPECT * 0.3125 + 20)
        expect(infoOffset(90, 100).x).toBeLessThan(0)
    })

    it('says how many screens away, from a tenth up', () => {
        expect(formatScreens(null)).toBeNull()
        expect(formatScreens(0.05)).toBeNull()
        expect(formatScreens(0.5)).toBe('50% screen')
        expect(formatScreens(2.5)).toBe('2.5× screen')
        expect(formatScreens(12.4)).toBe('12× screen')
    })
})
