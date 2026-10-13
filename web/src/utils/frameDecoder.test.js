import {decodeFrame, decodeRgb8Jpeg, SA09_HEADER_SIZE} from './frameDecoder.js'
import {SA10_MAGIC} from '../constants'
import {
    createChunkedTestFrame,
    createInvalidMagicBuffer,
    createTestFrame,
} from '../composables/__tests__/webSocketTestUtils.js'

function jpegFrame(width, height, payload) {
    const buffer = new ArrayBuffer(16 + payload.length)
    const view = new DataView(buffer)
    view.setUint32(0, SA10_MAGIC, true)
    view.setUint32(4, width, true)
    view.setUint32(8, height, true)
    view.setUint32(12, payload.length, true)
    new Uint8Array(buffer, 16).set(payload)
    return buffer
}

describe('decodeFrame', () => {
    afterEach(() => {
        vi.restoreAllMocks()
    })

    // Large enough to compress: LZ4 leaves a few bytes uncompressed and the helpers do not.
    it('decodes a single-block RGB8 frame (SA08)', () => {
        const frame = decodeFrame(createTestFrame(32, 32, 77))
        expect(frame).toMatchObject({width: 32, height: 32})
        expect(frame.frameData.length).toBe(32 * 32 * 3)
        expect(frame.frameData.every((byte) => byte === 77)).toBe(true)
    })

    it.each([1, 2, 3])('reassembles a chunked RGB8 frame from %i chunk(s) (SA09)', (chunks) => {
        const frame = decodeFrame(createChunkedTestFrame(32, 24, chunks, 42))
        expect(frame).toMatchObject({width: 32, height: 24})
        expect(frame.frameData.length).toBe(32 * 24 * 3)
        expect(frame.frameData.every((byte) => byte === 42)).toBe(true)
    })

    it('hands a JPEG frame (SA10) on as a blob', () => {
        const frame = decodeFrame(jpegFrame(640, 480, [0xff, 0xd8, 0xff, 0xd9]))
        expect(frame).toMatchObject({width: 640, height: 480, isJpeg: true})
        expect(frame.frameData).toBeInstanceOf(Blob)
        expect(frame.frameData.size).toBe(4)
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

    it('drops a chunked frame whose chunk runs past its end', () => {
        const error = vi.spyOn(console, 'error').mockImplementation(() => {})
        const buffer = createChunkedTestFrame(32, 32, 2, 9)
        new DataView(buffer).setUint32(SA09_HEADER_SIZE, buffer.byteLength, true)
        expect(decodeFrame(buffer)).toBeNull()
        expect(error).toHaveBeenCalled()
    })
})
