import {describe, it, expect} from 'vitest'
import {rgb8ToRgba8} from './pixelConversion.js'

describe('pixelConversion', () => {
    describe('rgb8ToRgba8', () => {
        it('returns a Uint8ClampedArray', () => {
            const rgb8 = new Uint8Array([0, 0, 0])
            const result = rgb8ToRgba8(rgb8, 1, 1)
            expect(result).toBeInstanceOf(Uint8ClampedArray)
        })

        it('converts a simple color', () => {
            const rgb8 = new Uint8Array([255, 0, 0])
            const result = rgb8ToRgba8(rgb8, 1, 1)
            expect(Array.from(result)).toEqual([255, 0, 0, 255])
        })

        it('handles black pixels', () => {
            const rgb8 = new Uint8Array([0, 0, 0])
            const result = rgb8ToRgba8(rgb8, 1, 1)
            expect(Array.from(result)).toEqual([0, 0, 0, 255])
        })

        it('converts multiple pixels', () => {
            const rgb8 = new Uint8Array(4 * 3) // 4 pixels * 3 bytes
            const result = rgb8ToRgba8(rgb8, 2, 2)
            expect(result.length).toBe(16) // 4 pixels * 4 bytes
        })

        it('preserves all channel values', () => {
            const rgb8 = new Uint8Array([128, 64, 200])
            const result = rgb8ToRgba8(rgb8, 1, 1)
            expect(Array.from(result)).toEqual([128, 64, 200, 255])
        })
    })
})
