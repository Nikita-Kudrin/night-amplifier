import {describe, it, expect} from 'vitest'
import {rgb8ToRgba8} from './pixelConversion.js'

describe('pixelConversion', () => {
    describe('rgb8ToRgba8', () => {
        it('writes into the buffer it is given', () => {
            const rgba8 = new Uint8ClampedArray(4)
            expect(rgb8ToRgba8(new Uint8Array([0, 0, 0]), rgba8)).toBe(rgba8)
        })

        it('copies every channel and makes the pixel opaque', () => {
            const rgb8 = new Uint8Array([255, 0, 0, 128, 64, 200])
            const rgba8 = rgb8ToRgba8(rgb8, new Uint8ClampedArray(8))
            expect(Array.from(rgba8)).toEqual([255, 0, 0, 255, 128, 64, 200, 255])
        })

        it('overwrites what the previous frame left in the buffer', () => {
            const rgba8 = new Uint8ClampedArray([9, 9, 9, 9])
            rgb8ToRgba8(new Uint8Array([1, 2, 3]), rgba8)
            expect(Array.from(rgba8)).toEqual([1, 2, 3, 255])
        })

        it('stops at the shorter of source and destination', () => {
            const rgba8 = new Uint8ClampedArray(4)
            rgb8ToRgba8(new Uint8Array([1, 2, 3, 4, 5, 6]), rgba8)
            expect(Array.from(rgba8)).toEqual([1, 2, 3, 255])
        })
    })
})
