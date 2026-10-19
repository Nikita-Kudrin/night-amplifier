import {describe, it, expect} from 'vitest'
import {downscaleRgb8, fitWithinEdge, rgb8ToRgba8} from './pixelConversion'

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

describe('fitWithinEdge', () => {
    it('fits the long side to the edge with the aspect kept, rounding down', () => {
        expect(fitWithinEdge(6248, 4176, 3840)).toEqual({width: 3840, height: 2566})
        expect(fitWithinEdge(4176, 6248, 3840)).toEqual({width: 2566, height: 3840})
    })

    it('never enlarges', () => {
        expect(fitWithinEdge(3008, 3008, 3840)).toEqual({width: 3008, height: 3008})
    })
})

describe('downscaleRgb8', () => {
    it('leaves a frame within the edge as it is', () => {
        const rgb = new Uint8Array(4 * 2 * 3)
        expect(downscaleRgb8(rgb, 4, 2, 4).data).toBe(rgb)
    })

    it('averages each whole block of every channel', () => {
        // 4x2 into a 2-px edge: factor 2, one row of two pixels.
        const rgb = new Uint8Array([
            0, 10, 200, 4, 20, 200, 100, 0, 0, 100, 0, 0,
            8, 30, 200, 12, 40, 201, 100, 0, 0, 101, 255, 255,
        ])
        const small = downscaleRgb8(rgb, 4, 2, 2)
        expect([small.width, small.height]).toEqual([2, 1])
        expect(Array.from(small.data)).toEqual([6, 25, 200, 100, 64, 64])
    })

    it('takes the smallest whole factor that fits, dropping the leftover edge', () => {
        const small = downscaleRgb8(new Uint8Array(6248 * 4 * 3), 6248, 4, 3840)
        expect([small.width, small.height]).toEqual([3124, 2])
    })
})
