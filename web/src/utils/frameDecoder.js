import lz4 from 'lz4js'
import {RGB8_MAGIC, RGB8_CHUNKED_MAGIC} from '../constants'

/**
 * Decode a binary WebSocket frame, dispatching by magic number.
 *
 * @param {ArrayBuffer} buffer - Raw binary data from WebSocket
 * @returns {object|null} Decoded frame { width, height, frameData } or null if invalid
 */
export function decodeFrame(buffer) {
    if (buffer.byteLength < 16) return null

    const view = new DataView(buffer)
    const magic = view.getUint32(0, true)

    if (magic === RGB8_CHUNKED_MAGIC) {
        return decodeRgb8Lz4Chunked(buffer, view)
    }

    if (magic === RGB8_MAGIC) {
        return decodeRgb8Lz4(buffer, view)
    }

    console.error('Unknown frame magic number:', magic.toString(16))
    return null
}

/**
 * Decode RGB8+LZ4 binary stream format (SA08 — legacy single-block)
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
 * @param {DataView} view - DataView over buffer
 * @returns {object|null} Decoded frame { width, height, frameData } or null if invalid
 */
export function decodeRgb8Lz4(buffer, view) {
    if (!view) view = new DataView(buffer)

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
        console.error('SA08 decompression failed:', e)
        return null
    }

    return {width, height, frameData: decompressedBuffer}
}

const SA09_HEADER_SIZE = 20
const SA09_CHUNK_DESCRIPTOR_SIZE = 8

/**
 * Decode chunked RGB8+LZ4 binary stream format (SA09)
 *
 * Header (20 bytes):
 * - bytes 0-3:    Magic "SA09" (0x53413039)
 * - bytes 4-7:    Width (u32 LE)
 * - bytes 8-11:   Height (u32 LE)
 * - bytes 12-15:  Total payload size (u32 LE)
 * - bytes 16-19:  Chunk count (u32 LE)
 *
 * Per-chunk descriptor (8 bytes each):
 * - bytes 0-3:    Compressed size (u32 LE)
 * - bytes 4-7:    Decompressed size (u32 LE)
 *
 * Followed by concatenated compressed chunk data
 *
 * @param {ArrayBuffer} buffer - Raw binary data from WebSocket
 * @param {DataView} view - DataView over buffer
 * @returns {object|null} Decoded frame { width, height, frameData } or null if invalid
 */
export function decodeRgb8Lz4Chunked(buffer, view) {
    if (!view) view = new DataView(buffer)

    if (buffer.byteLength < SA09_HEADER_SIZE) return null

    const width = view.getUint32(4, true)
    const height = view.getUint32(8, true)
    const chunkCount = view.getUint32(16, true)

    const totalDecompressed = width * height * 3
    const output = lz4.makeBuffer(totalDecompressed)

    const descriptorsSize = chunkCount * SA09_CHUNK_DESCRIPTOR_SIZE
    let dataOffset = SA09_HEADER_SIZE + descriptorsSize
    let outputOffset = 0

    try {
        for (let i = 0; i < chunkCount; i++) {
            const descOffset = SA09_HEADER_SIZE + i * SA09_CHUNK_DESCRIPTOR_SIZE
            const compressedSize = view.getUint32(descOffset, true)
            const decompressedSize = view.getUint32(descOffset + 4, true)

            const chunkData = new Uint8Array(buffer, dataOffset, compressedSize)
            lz4.decompressBlock(chunkData, output, 0, compressedSize, outputOffset)

            outputOffset += decompressedSize
            dataOffset += compressedSize
        }
    } catch (e) {
        console.error('SA09 decompression failed:', e)
        return null
    }

    return {width, height, frameData: output}
}
