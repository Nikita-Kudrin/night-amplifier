import {fieldOfViewDeg} from './optics.js'

describe('fieldOfViewDeg', () => {
    // The case Pro's `push_to/config.rs` pins: 4000 px of 3.76 µm on 200 mm.
    it('matches the solver for a deep-sky rig', () => {
        expect(fieldOfViewDeg(4000, 3.76, 200)).toBeCloseTo(4.3066, 3)
    })

    it('applies a barlow or reducer to the focal length', () => {
        expect(fieldOfViewDeg(3008, 3.76, 400, 2)).toBeCloseTo(fieldOfViewDeg(3008, 3.76, 800) ?? Number.NaN, 10)
        expect(fieldOfViewDeg(3008, 3.76, 400, 0.5)).toBeCloseTo(fieldOfViewDeg(3008, 3.76, 200) ?? Number.NaN, 10)
    })

    // A small-angle shortcut overstated a fisheye's field: 2·atan never passes 180°.
    it('stays exact for very wide fields', () => {
        expect(fieldOfViewDeg(5000, 4, 10)).toBeCloseTo(90, 0)
        expect(fieldOfViewDeg(1e6, 4, 10)).toBeLessThan(180)
    })

    it('treats a missing barlow as none', () => {
        expect(fieldOfViewDeg(1000, 4, 500, null)).toBe(fieldOfViewDeg(1000, 4, 500))
    })

    it.each([
        [null, 3.76, 400],
        [3008, 0, 400],
        [3008, 3.76, undefined],
        [-1, 3.76, 400],
    ])('is null without a usable input (%s, %s, %s)', (px, um, fl) => {
        expect(fieldOfViewDeg(px, um, fl)).toBeNull()
    })
})
