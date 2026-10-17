import {assert} from 'vitest'
import {decodeFrame, decodeRgb8Jpeg, SA09_HEADER_SIZE, SA10_MAGIC} from './frameDecoder'
import type {DecodedFrame} from './frameDecoder'
import {
    createChunkedTestFrame,
    createInvalidMagicBuffer,
    createTestFrame,
} from '../composables/__tests__/webSocketTestUtils'

function jpegFrame(width: number, height: number, payload: number[]): ArrayBuffer {
    const buffer = new ArrayBuffer(16 + payload.length)
    const view = new DataView(buffer)
    view.setUint32(0, SA10_MAGIC, true)
    view.setUint32(4, width, true)
    view.setUint32(8, height, true)
    view.setUint32(12, payload.length, true)
    new Uint8Array(buffer, 16).set(payload)
    return buffer
}

/** The pixels of a lossless frame; fails on anything else. */
function pixels(frame: DecodedFrame | null): Uint8Array {
    assert(frame && !frame.isJpeg, 'a lossless frame')
    return frame.frameData
}

describe('decodeFrame', () => {
    afterEach(() => {
        vi.restoreAllMocks()
    })

    // Large enough to compress: LZ4 leaves a few bytes uncompressed and the helpers do not.
    it('decodes a single-block RGB8 frame (SA08)', () => {
        const frame = decodeFrame(createTestFrame(32, 32, 77))
        expect(frame).toMatchObject({width: 32, height: 32})
        expect(pixels(frame).length).toBe(32 * 32 * 3)
        expect(pixels(frame).every((byte) => byte === 77)).toBe(true)
    })

    it.each([1, 2, 3])('reassembles a chunked RGB8 frame from %i chunk(s) (SA09)', (chunks) => {
        const frame = decodeFrame(createChunkedTestFrame(32, 24, chunks, 42))
        expect(frame).toMatchObject({width: 32, height: 24})
        expect(pixels(frame).length).toBe(32 * 24 * 3)
        expect(pixels(frame).every((byte) => byte === 42)).toBe(true)
    })

    it('hands a JPEG frame (SA10) on as a blob', () => {
        const frame = decodeFrame(jpegFrame(640, 480, [0xff, 0xd8, 0xff, 0xd9]))
        expect(frame).toMatchObject({width: 640, height: 480, isJpeg: true})
        assert(frame?.isJpeg)
        expect(frame.frameData).toBeInstanceOf(Blob)
        expect(frame.frameData.size).toBe(4)
    })

    it('types a JPEG frame apart, so its blob is never read as pixels', () => {
        const frame = decodeFrame(jpegFrame(1, 1, [0xff, 0xd8]))
        // @ts-expect-error: the frame may be a JPEG blob, which has no pixels to count
        expect(frame?.frameData.length).toBeUndefined()
    })

    it('refuses a JPEG frame shorter than its header says', () => {
        const buffer = jpegFrame(640, 480, [0xff, 0xd8])
        new DataView(buffer).setUint32(12, 99, true)
        expect(decodeRgb8Jpeg(buffer)).toBeNull()
    })

    it('ignores a frame too short to carry a header', () => {
        expect(decodeFrame(new ArrayBuffer(8))).toBeNull()
    })

    it('reports and drops an unknown format', () => {
        const error = vi.spyOn(console, 'error').mockImplementation(() => {})
        expect(decodeFrame(createInvalidMagicBuffer())).toBeNull()
        expect(error).toHaveBeenCalled()
    })

    it('decodes the smallest frames, which LZ4 cannot shrink', () => {
        expect(pixels(decodeFrame(createTestFrame(2, 2, 5)))).toEqual(new Uint8Array(12).fill(5))
        expect(pixels(decodeFrame(createChunkedTestFrame(2, 2, 2, 5)))).toEqual(new Uint8Array(12).fill(5))
    })

    describe('drops a frame its own header contradicts, rather than throwing', () => {
        beforeEach(() => vi.spyOn(console, 'error').mockImplementation(() => {}))

        const truncated = (buffer: ArrayBuffer, bytes: number) => buffer.slice(0, buffer.byteLength - bytes)

        it.each([
            ['SA08 cut short', () => truncated(createTestFrame(32, 32), 3)],
            ['SA08 under 4 payload bytes', () => {
                const buffer = createTestFrame(32, 32)
                new DataView(buffer).setUint32(12, 2, true)
                return buffer
            }],
            ['SA08 naming another size', () => {
                const buffer = createTestFrame(32, 32)
                new DataView(buffer).setUint32(4, 64, true)
                return buffer
            }],
            ['SA09 cut short', () => truncated(createChunkedTestFrame(32, 32, 3), 3)],
            ['SA09 cut inside its descriptors', () => createChunkedTestFrame(32, 32, 3).slice(0, SA09_HEADER_SIZE + 12)],
            ['SA09 claiming a million chunks', () => {
                const buffer = createChunkedTestFrame(32, 32, 2)
                new DataView(buffer).setUint32(16, 1_000_000, true)
                return buffer
            }],
            ['SA09 naming another size', () => {
                const buffer = createChunkedTestFrame(32, 32, 2)
                new DataView(buffer).setUint32(8, 64, true)
                return buffer
            }],
            ['SA09 whose chunk spills into the next stripe', () => {
                const buffer = createChunkedTestFrame(32, 32, 2)
                const view = new DataView(buffer)
                const first = view.getUint32(SA09_HEADER_SIZE + 4, true)
                view.setUint32(SA09_HEADER_SIZE + 4, first - 3, true)
                view.setUint32(SA09_HEADER_SIZE + 12, view.getUint32(SA09_HEADER_SIZE + 12, true) + 3, true)
                return buffer
            }],
        ])('%s', (_, frame) => {
            expect(decodeFrame(frame())).toBeNull()
            expect(console.error).toHaveBeenCalled()
        })

        it('never allocates the pixels a contradicted header names', () => {
            const buffer = createChunkedTestFrame(32, 32, 2)
            const view = new DataView(buffer)
            view.setUint32(4, 60_000, true)
            view.setUint32(8, 60_000, true)

            expect(() => decodeFrame(buffer)).not.toThrow()
            expect(decodeFrame(buffer)).toBeNull()
        })
    })

    it('drops a chunked frame whose chunk runs past its end', () => {
        const error = vi.spyOn(console, 'error').mockImplementation(() => {})
        const buffer = createChunkedTestFrame(32, 32, 2, 9)
        new DataView(buffer).setUint32(SA09_HEADER_SIZE, buffer.byteLength, true)
        expect(decodeFrame(buffer)).toBeNull()
        expect(error).toHaveBeenCalled()
    })
})
