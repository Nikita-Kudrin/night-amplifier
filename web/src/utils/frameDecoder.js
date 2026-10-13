import lz4 from 'lz4js'

// The stream formats' magic numbers: "SA08".."SA10" read as a little-endian u32.
export const RGB8_MAGIC = 0x53413038 // "SA08" — legacy single-block
export const RGB8_CHUNKED_MAGIC = 0x53413039 // "SA09" — parallel chunked
export const SA10_MAGIC = 0x53413130 // "SA10" — dynamic resolution JPEG

/** The SA08 and SA10 header: magic, width, height and payload size, each a u32. */
export const FRAME_HEADER_SIZE = 16
export const SA09_HEADER_SIZE = 20
export const SA09_CHUNK_DESCRIPTOR_SIZE = 8

/**
 * Logs and drops a frame whose header disagrees with its own bytes. Unchecked, a
 * truncated frame threw a `RangeError` inside the socket's async `onMessage`.
 */
function malformed(format, reason) {
    console.error(`${format} frame dropped: ${reason}`)
    return null
}

/**
 * Decode a binary WebSocket frame, dispatching by magic number.
 *
 * @param {ArrayBuffer} buffer - Raw binary data from WebSocket
 * @returns {object|null} Decoded frame { width, height, frameData } or null if invalid
 */
export function decodeFrame(buffer) {
    if (buffer.byteLength < FRAME_HEADER_SIZE) return null

    const view = new DataView(buffer)
    const magic = view.getUint32(0, true)

    if (magic === RGB8_CHUNKED_MAGIC) {
        return decodeRgb8Lz4Chunked(buffer, view)
    }

    if (magic === RGB8_MAGIC) {
        return decodeRgb8Lz4(buffer, view)
    }

    if (magic === SA10_MAGIC) {
        return decodeRgb8Jpeg(buffer, view)
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

    if (compressedSize < 4 || FRAME_HEADER_SIZE + compressedSize > buffer.byteLength) {
        return malformed('SA08', `payload of ${compressedSize} bytes in a ${buffer.byteLength}-byte frame`)
    }
    // The payload opens with its decompressed size, a u32.
    const decompressedSize = view.getUint32(FRAME_HEADER_SIZE, true)
    if (decompressedSize !== width * height * 3) {
        return malformed('SA08', `${decompressedSize} bytes of pixels for ${width}x${height}`)
    }

    const lz4BlockData = new Uint8Array(buffer, FRAME_HEADER_SIZE + 4, compressedSize - 4)

    let decompressedBuffer
    try {
        decompressedBuffer = lz4.makeBuffer(decompressedSize)
        const end = lz4.decompressBlock(lz4BlockData, decompressedBuffer, 0, lz4BlockData.length, 0)
        if (end !== decompressedSize) return malformed('SA08', `decompressed to ${end} of ${decompressedSize} bytes`)
    } catch (e) {
        console.error('SA08 decompression failed:', e)
        return null
    }

    return {width, height, frameData: decompressedBuffer}
}

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

    // Every descriptor is checked before the output is allocated from the header's size.
    const descriptorsSize = chunkCount * SA09_CHUNK_DESCRIPTOR_SIZE
    if (SA09_HEADER_SIZE + descriptorsSize > buffer.byteLength) {
        return malformed('SA09', `${chunkCount} chunk descriptors in a ${buffer.byteLength}-byte frame`)
    }
    const chunks = []
    let dataEnd = SA09_HEADER_SIZE + descriptorsSize
    let declared = 0
    for (let i = 0; i < chunkCount; i++) {
        const descOffset = SA09_HEADER_SIZE + i * SA09_CHUNK_DESCRIPTOR_SIZE
        const chunk = {
            offset: dataEnd,
            compressedSize: view.getUint32(descOffset, true),
            decompressedSize: view.getUint32(descOffset + 4, true),
        }
        dataEnd += chunk.compressedSize
        declared += chunk.decompressedSize
        chunks.push(chunk)
    }
    if (dataEnd > buffer.byteLength) {
        return malformed('SA09', `chunks run to byte ${dataEnd} of ${buffer.byteLength}`)
    }
    if (declared !== totalDecompressed) {
        return malformed('SA09', `chunks hold ${declared} bytes of pixels for ${width}x${height}`)
    }

    const output = lz4.makeBuffer(totalDecompressed)
    let outputOffset = 0

    try {
        for (const {offset, compressedSize, decompressedSize} of chunks) {
            const chunkData = new Uint8Array(buffer, offset, compressedSize)
            const end = lz4.decompressBlock(chunkData, output, 0, compressedSize, outputOffset)
            // A chunk spilling over its stripe would land in the next one's rows.
            if (end !== outputOffset + decompressedSize) {
                return malformed('SA09', `a chunk decompressed to ${end - outputOffset} of ${decompressedSize} bytes`)
            }
            outputOffset = end
        }
    } catch (e) {
        console.error('SA09 decompression failed:', e)
        return null
    }

    return {width, height, frameData: output}
}

/**
 * Decode dynamic resolution JPEG binary stream format (SA10)
 *
 * Header (16 bytes):
 * - bytes 0-3:    Magic "SA10" (0x53413130 LE)
 * - bytes 4-7:    Width (u32 LE)
 * - bytes 8-11:   Height (u32 LE)
 * - bytes 12-15:  Payload size (u32 LE)
 *
 * Followed by raw JPEG data
 *
 * @param {ArrayBuffer} buffer - Raw binary data from WebSocket
 * @param {DataView} view - DataView over buffer
 * @returns {object|null} Decoded frame { width, height, frameData: Blob, isJpeg: true } or null if invalid
 */
export function decodeRgb8Jpeg(buffer, view) {
    if (!view) view = new DataView(buffer)
    if (buffer.byteLength < FRAME_HEADER_SIZE) return null

    const width = view.getUint32(4, true)
    const height = view.getUint32(8, true)
    const payloadSize = view.getUint32(12, true)

    if (buffer.byteLength < FRAME_HEADER_SIZE + payloadSize) return null

    const jpegData = new Uint8Array(buffer, FRAME_HEADER_SIZE, payloadSize)
    const blob = new Blob([jpegData], {type: 'image/jpeg'})

    return {width, height, frameData: blob, isJpeg: true}
}
