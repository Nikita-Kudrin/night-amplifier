import {nextTick} from 'vue'

// --- Constants ---
export const WS_STATES = {
    CONNECTING: 0,
    OPEN: 1,
    CLOSING: 2,
    CLOSED: 3,
}

export const RGB8_MAGIC = 0x53413038 // "SA08"
export const RGB8_HEADER_SIZE = 16
export const RGB8_BYTES_PER_PIXEL = 3

// --- MockWebSocket ---
export class MockWebSocket {
    static CONNECTING = WS_STATES.CONNECTING
    static OPEN = WS_STATES.OPEN
    static CLOSING = WS_STATES.CLOSING
    static CLOSED = WS_STATES.CLOSED
    static instances = []
    static lastSent = null

    constructor(url) {
        this.url = url
        this.readyState = MockWebSocket.CONNECTING
        this.onopen = null
        this.onclose = null
        this.onerror = null
        this.onmessage = null
        MockWebSocket.instances.push(this)
    }

    send(data) {
        MockWebSocket.lastSent = data
    }

    close() {
        this.readyState = MockWebSocket.CLOSED
        this.onclose?.({code: 1000})
    }

    simulateOpen() {
        this.readyState = MockWebSocket.OPEN
        this.onopen?.()
    }

    simulateMessage(data) {
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

    static get latest() {
        return MockWebSocket.instances[MockWebSocket.instances.length - 1]
    }

    static get first() {
        return MockWebSocket.instances[0]
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

export async function sendEvent(eventData) {
    getWebSocket().simulateMessage(JSON.stringify(eventData))
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
export function createRgb8PixelData(width, height, fillValue = 100) {
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

export function createRgb8Lz4Buffer(width, height, rgb8Data) {
    const lz4 = require('lz4js')

    const maxCompressedSize = lz4.compressBound(rgb8Data.length)
    const compressedBlock = lz4.makeBuffer(maxCompressedSize)
    const actualCompressedSize = lz4.compressBlock(
        rgb8Data,
        compressedBlock,
        0,
        rgb8Data.length,
        []
    )

    const decompressedSize = rgb8Data.length
    const payloadSize = 4 + actualCompressedSize
    const payload = new Uint8Array(payloadSize)

    payload[0] = decompressedSize & 0xff
    payload[1] = (decompressedSize >> 8) & 0xff
    payload[2] = (decompressedSize >> 16) & 0xff
    payload[3] = (decompressedSize >> 24) & 0xff
    payload.set(compressedBlock.subarray(0, actualCompressedSize), 4)

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

// --- Setup Global Mock ---
export function setupGlobalWebSocketMock() {
    global.WebSocket = MockWebSocket
}
