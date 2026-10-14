import {nextTick} from 'vue'
import lz4 from 'lz4js'
import {beforeEach, vi} from 'vitest'
import {
    FRAME_HEADER_SIZE as RGB8_HEADER_SIZE,
    RGB8_CHUNKED_MAGIC,
    RGB8_MAGIC,
    SA09_CHUNK_DESCRIPTOR_SIZE,
    SA09_HEADER_SIZE,
} from '../../utils/frameDecoder'
import {serverEvent} from '../../test/eventStream'
import type {EventFields} from '../../test/eventStream'
import type {ServerEventType, SyncedEvent} from '../api.events'

// --- Constants ---
const WS_STATES = {
    CONNECTING: 0,
    OPEN: 1,
    CLOSING: 2,
    CLOSED: 3,
}

export const RGB8_BYTES_PER_PIXEL = 3

type CloseInit = {code: number; reason?: string}

// --- MockWebSocket ---
/** The `WebSocket` a test drives by hand: `simulate*` play the server's side. */
export class MockWebSocket {
    static CONNECTING = WS_STATES.CONNECTING
    static OPEN = WS_STATES.OPEN
    static CLOSING = WS_STATES.CLOSING
    static CLOSED = WS_STATES.CLOSED
    static instances: MockWebSocket[] = []
    static lastSent: unknown = null

    url: string
    readyState: number = MockWebSocket.CONNECTING
    onopen: (() => void) | null = null
    onclose: ((event: CloseInit) => void) | null = null
    onerror: ((event: Event) => void) | null = null
    onmessage: ((event: {data: unknown}) => void) | null = null
    closedWith?: CloseInit

    constructor(url: string) {
        this.url = url
        MockWebSocket.instances.push(this)
    }

    send(data: unknown) {
        MockWebSocket.lastSent = data
    }

    close(code = 1000, reason = '') {
        this.readyState = MockWebSocket.CLOSED
        this.closedWith = {code, reason}
        this.onclose?.({code, reason})
    }

    simulateOpen() {
        this.readyState = MockWebSocket.OPEN
        this.onopen?.()
    }

    simulateMessage(data: unknown) {
        this.onmessage?.({data})
    }

    simulateError() {
        this.onerror?.(new Event('error'))
    }

    simulateClose() {
        this.readyState = MockWebSocket.CLOSED
        this.onclose?.({code: 1000})
    }

    static reset() {
        MockWebSocket.instances = []
        MockWebSocket.lastSent = null
    }

    static get latest(): MockWebSocket {
        const socket = MockWebSocket.instances.at(-1)
        if (!socket) throw new Error('no socket was opened')
        return socket
    }

    static get first(): MockWebSocket {
        const socket = MockWebSocket.instances[0]
        if (!socket) throw new Error('no socket was opened')
        return socket
    }
}

// --- Test Context Helpers ---
export function createTestContext() {
    MockWebSocket.reset()
    vi.useFakeTimers()
}

export function createTestContextWithoutTimers() {
    MockWebSocket.reset()
}

export function cleanupTestContext() {
    vi.useRealTimers()
    vi.restoreAllMocks()
}

export function cleanupTestContextWithoutTimers() {
    vi.restoreAllMocks()
}

export function getWebSocket() {
    return MockWebSocket.first
}

export async function openWebSocket() {
    getWebSocket().simulateOpen()
    await nextTick()
}

/** Sends `eventData`, held to the server's catalogue (`serverEvent`), on the first socket. */
export async function sendEvent<K extends ServerEventType>(eventData: EventFields<K>) {
    getWebSocket().simulateMessage(JSON.stringify(serverEvent(eventData.type, eventData)))
    await nextTick()
}

/** Ends a sync on the newest socket, as the server's `synced` does after its snapshot. */
export async function syncWebSocket({cause = 'connect', dropped = 0}: Partial<SyncedEvent> = {}) {
    MockWebSocket.latest.simulateMessage(JSON.stringify({type: 'synced', cause, dropped}))
    await nextTick()
}

export async function waitForAsyncProcessing() {
    await new Promise((resolve) => setTimeout(resolve, 10))
    await nextTick()
}

export function suppressConsoleErrors() {
    return vi.spyOn(console, 'error').mockImplementation(() => {
    })
}

// --- RGB8 Frame Helpers ---
export function createRgb8PixelData(width: number, height: number, fillValue = 100) {
    const pixelCount = width * height
    const data = new Uint8Array(pixelCount * RGB8_BYTES_PER_PIXEL)

    for (let i = 0; i < pixelCount; i++) {
        const offset = i * RGB8_BYTES_PER_PIXEL
        const val = fillValue & 0xff
        data[offset] = val     // R
        data[offset + 1] = val // G
        data[offset + 2] = val // B
    }

    return data
}

/**
 * An LZ4 block of `data`. lz4js answers 0 for input it cannot shrink (a 2x2 frame), where
 * a real encoder, the server's among them, writes the bytes as one literal run.
 */
function lz4Block(data: Uint8Array) {
    const block = lz4.makeBuffer(lz4.compressBound(data.length))
    const size = lz4.compressBlock(data, block, 0, data.length, [])
    if (size > 0) return block.subarray(0, size)

    const lengthBytes: number[] = []
    if (data.length >= 15) {
        let rest = data.length - 15
        for (; rest >= 255; rest -= 255) lengthBytes.push(255)
        lengthBytes.push(rest)
    }
    return new Uint8Array([Math.min(data.length, 15) << 4, ...lengthBytes, ...data])
}

export function createRgb8Lz4Buffer(width: number, height: number, rgb8Data: Uint8Array) {
    const compressedBlock = lz4Block(rgb8Data)
    const actualCompressedSize = compressedBlock.length

    const decompressedSize = rgb8Data.length
    const payloadSize = 4 + actualCompressedSize
    const payload = new Uint8Array(payloadSize)

    payload[0] = decompressedSize & 0xff
    payload[1] = (decompressedSize >> 8) & 0xff
    payload[2] = (decompressedSize >> 16) & 0xff
    payload[3] = (decompressedSize >> 24) & 0xff
    payload.set(compressedBlock, 4)

    const header = new ArrayBuffer(RGB8_HEADER_SIZE)
    const headerView = new DataView(header)
    headerView.setUint32(0, RGB8_MAGIC, true)
    headerView.setUint32(4, width, true)
    headerView.setUint32(8, height, true)
    headerView.setUint32(12, payloadSize, true)

    const result = new Uint8Array(RGB8_HEADER_SIZE + payloadSize)
    result.set(new Uint8Array(header), 0)
    result.set(payload, RGB8_HEADER_SIZE)

    return result.buffer
}

export function createTestFrame(width = 2, height = 2, fillValue = 100) {
    const rgb8Data = createRgb8PixelData(width, height, fillValue)
    return createRgb8Lz4Buffer(width, height, rgb8Data)
}

export function createInvalidMagicBuffer() {
    const buffer = new ArrayBuffer(32)
    const view = new DataView(buffer)
    view.setUint32(0, 0x12345678, true)
    view.setUint32(4, 2, true)
    view.setUint32(8, 2, true)
    view.setUint32(12, 16, true)
    return buffer
}

export function createChunkedTestFrame(width = 4, height = 4, chunkCount = 2, fillValue = 100) {
    const rgb8Data = createRgb8PixelData(width, height, fillValue)
    const rowBytes = width * RGB8_BYTES_PER_PIXEL
    const totalRows = height
    const rowsPerChunk = Math.floor(totalRows / chunkCount)
    const remainderRows = totalRows % chunkCount

    const chunks: {compressed: Uint8Array; decompressedSize: number}[] = []
    let rowOffset = 0
    for (let i = 0; i < chunkCount; i++) {
        const rows = rowsPerChunk + (i < remainderRows ? 1 : 0)
        const start = rowOffset * rowBytes
        const end = (rowOffset + rows) * rowBytes
        const stripe = rgb8Data.subarray(start, end)

        chunks.push({compressed: lz4Block(stripe), decompressedSize: stripe.length})
        rowOffset += rows
    }

    const descriptorsSize = chunkCount * SA09_CHUNK_DESCRIPTOR_SIZE
    const compressedTotal = chunks.reduce((sum, c) => sum + c.compressed.length, 0)
    const payloadSize = descriptorsSize + compressedTotal
    const totalSize = SA09_HEADER_SIZE + payloadSize

    const result = new ArrayBuffer(totalSize)
    const view = new DataView(result)

    view.setUint32(0, RGB8_CHUNKED_MAGIC, true)
    view.setUint32(4, width, true)
    view.setUint32(8, height, true)
    view.setUint32(12, payloadSize, true)
    view.setUint32(16, chunkCount, true)

    let descOffset = SA09_HEADER_SIZE
    let dataOffset = SA09_HEADER_SIZE + descriptorsSize
    const bytes = new Uint8Array(result)

    for (const chunk of chunks) {
        view.setUint32(descOffset, chunk.compressed.length, true)
        view.setUint32(descOffset + 4, chunk.decompressedSize, true)
        descOffset += SA09_CHUNK_DESCRIPTOR_SIZE

        bytes.set(chunk.compressed, dataOffset)
        dataOffset += chunk.compressed.length
    }

    return result
}

// --- Setup Global Mock ---
/** Puts `MockWebSocket` in place before each test of the calling file; `unstubGlobals` takes it back after. */
export function setupGlobalWebSocketMock() {
    beforeEach(() => vi.stubGlobal('WebSocket', MockWebSocket))
}
