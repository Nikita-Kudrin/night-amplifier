import lz4 from 'lz4js'
import {RGB8_MAGIC} from '../constants'

/**
 * Decode RGB8+LZ4 binary stream format
 *
 * Header layout (16 bytes):
 * - bytes 0-3:   Magic number "SA08" (0x53413038)
 * - bytes 4-7:   Width (u32, little-endian)
 * - bytes 8-11:  Height (u32, little-endian)
 * - bytes 12-15: Compressed size (u32, little-endian)
 *
 * Followed by LZ4-compressed RGB8 pixel data (3 bytes per pixel)
 *
 * @param {ArrayBuffer} buffer - Raw binary data from WebSocket
 * @returns {object|null} Decoded frame { width, height, frameData } or null if invalid
 */
export function decodeRgb8Lz4(buffer) {
    const view = new DataView(buffer)

    if (buffer.byteLength < 16) return null;

    const magic = view.getUint32(0, true)
    if (magic !== RGB8_MAGIC) {
        console.error('Invalid magic number, expected SA08:', magic.toString(16))
        return null
    }

    const width = view.getUint32(4, true)
    const height = view.getUint32(8, true)
    const compressedSize = view.getUint32(12, true)

    const compressedData = new Uint8Array(buffer, 16, compressedSize)
    const decompressedSize =
        compressedData[0] | (compressedData[1] << 8) |
        (compressedData[2] << 16) | (compressedData[3] << 24)

    const lz4BlockData = new Uint8Array(buffer, 20, compressedSize - 4)

    let decompressedBuffer
    try {
        decompressedBuffer = lz4.makeBuffer(decompressedSize)
        lz4.decompressBlock(lz4BlockData, decompressedBuffer, 0, lz4BlockData.length, 0)
    } catch (e) {
        console.error('Decompression failed:', e)
        return null
    }

    return {width, height, frameData: decompressedBuffer}
}
