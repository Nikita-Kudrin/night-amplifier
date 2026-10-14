import {describe, it, expect, expectTypeOf, vi, beforeEach, afterEach} from 'vitest'
import {nextTick} from 'vue'
import {
    MockWebSocket,
    createTestContext,
    cleanupTestContext,
    getWebSocket,
    openWebSocket,
    setupGlobalWebSocketMock,
} from './webSocketTestUtils'

setupGlobalWebSocketMock()

import {useWebSocket as originalUseWebSocket, WS_CLOSE_UNRESPONSIVE} from '../useWebSocket'
import type {SocketData, WebSocketConnection, WebSocketOptions} from '../useWebSocket'
import {WS_RECONNECT} from '../../constants'
import { mount } from '@vue/test-utils'
import type { VueWrapper } from '@vue/test-utils'

let currentApp: VueWrapper | null = null;
function useWebSocket(...args: Parameters<typeof originalUseWebSocket>): WebSocketConnection {
    let result: WebSocketConnection | undefined;
    currentApp = mount({
        setup() {
            result = originalUseWebSocket(...args)
            return () => {}
        }
    })
    if (!result) throw new Error('setup never ran')
    return result
}

describe('useWebSocket', () => {
    beforeEach(createTestContext)
    afterEach(() => {
        cleanupTestContext()
        if (currentApp) {
            currentApp.unmount()
            currentApp = null
        }
    })

    describe('connection management', () => {
        it('connects automatically when autoConnect is true', () => {
            useWebSocket('/ws/test', {autoConnect: true})

            expect(MockWebSocket.instances).toHaveLength(1)
            expect(getWebSocket().url).toContain('/ws/test')
        })

        it('does not connect when autoConnect is false', () => {
            const {connected} = useWebSocket('/ws/test', {autoConnect: false})

            expect(MockWebSocket.instances).toHaveLength(0)
            expect(connected.value).toBe(false)
        })

        it('sets connected to true on open', async () => {
            const {connected} = useWebSocket('/ws/test')
            await openWebSocket()

            expect(connected.value).toBe(true)
        })

        it('sets connected to false on close', async () => {
            const {connected} = useWebSocket('/ws/test')

            await openWebSocket()
            expect(connected.value).toBe(true)

            getWebSocket().simulateClose()
            await nextTick()
            expect(connected.value).toBe(false)
        })

        it.each([
            ['onOpen', 'simulateOpen'],
            ['onClose', 'simulateClose'],
            ['onError', 'simulateError'],
        ] as const)('calls %s callback on %s', async (callbackName, simulateMethod) => {
            const callback = vi.fn()
            useWebSocket('/ws/test', {[callbackName]: callback})

            if (simulateMethod !== 'simulateOpen') {
                getWebSocket().simulateOpen()
            }
            getWebSocket()[simulateMethod]()
            await nextTick()

            expect(callback).toHaveBeenCalled()
        })
    })

    describe('message handling', () => {
        it('calls onMessage callback with message event', async () => {
            const onMessage = vi.fn()
            useWebSocket('/ws/test', {onMessage})

            await openWebSocket()
            getWebSocket().simulateMessage('test data')
            await nextTick()

            expect(onMessage).toHaveBeenCalledWith(expect.objectContaining({data: 'test data'}))
        })

        it('ignores pong messages', async () => {
            const onMessage = vi.fn()
            useWebSocket('/ws/test', {onMessage})

            await openWebSocket()
            getWebSocket().simulateMessage('pong')
            await nextTick()

            expect(onMessage).not.toHaveBeenCalled()
        })

        it('hands onMessage what a browser socket delivers, or the text a caller says it gets', () => {
            expectTypeOf<SocketData>().toEqualTypeOf<ArrayBuffer | Blob | string>()
            expectTypeOf<Parameters<NonNullable<WebSocketOptions['onMessage']>>[0]>()
                .toEqualTypeOf<MessageEvent<ArrayBuffer | Blob | string>>()
            expectTypeOf<Parameters<NonNullable<WebSocketOptions<string>['onMessage']>>[0]>()
                .toEqualTypeOf<MessageEvent<string>>()
            // @ts-expect-error a socket carries text or binary, nothing else
            expectTypeOf<WebSocketOptions<number>>()
        })

        it('send() transmits data when connected', async () => {
            const {send} = useWebSocket('/ws/test')

            await openWebSocket()
            send('hello')

            expect(MockWebSocket.lastSent).toBe('hello')
        })
    })

    describe('reconnection', () => {
        it('attempts reconnection after disconnect when reconnect is true', () => {
            useWebSocket('/ws/test', {reconnect: true, reconnectInterval: 1000})

            getWebSocket().simulateOpen()
            getWebSocket().simulateClose()

            expect(MockWebSocket.instances).toHaveLength(1)

            vi.advanceTimersByTime(1000)

            expect(MockWebSocket.instances).toHaveLength(2)
        })

        it('does not reconnect when reconnect is false', () => {
            useWebSocket('/ws/test', {reconnect: false})

            getWebSocket().simulateOpen()
            getWebSocket().simulateClose()
            vi.advanceTimersByTime(5000)

            expect(MockWebSocket.instances).toHaveLength(1)
        })

        // The kiosk display has no keyboard to reload with, so giving up is the same as
        // staying black for the rest of the night.
        it('keeps reconnecting indefinitely', () => {
            useWebSocket('/ws/test', {
                reconnect: true,
                reconnectInterval: 100,
                maxReconnectInterval: 400,
            })

            // Well past the old ten-attempt cap.
            for (let i = 0; i < 20; i++) {
                MockWebSocket.instances[i].simulateClose()
                vi.advanceTimersByTime(400)
            }

            expect(MockWebSocket.instances).toHaveLength(21)
        })

        it('backs off exponentially up to the ceiling', () => {
            useWebSocket('/ws/test', {
                reconnect: true,
                reconnectInterval: 100,
                maxReconnectInterval: 400,
            })

            // 100, 200, 400, then 400 forever. Each step asserts the socket has *not*
            // appeared one tick early, so a flat interval would fail here.
            for (const delay of [100, 200, 400, 400]) {
                const before = MockWebSocket.instances.length
                MockWebSocket.instances[before - 1].simulateClose()

                vi.advanceTimersByTime(delay - 1)
                expect(MockWebSocket.instances).toHaveLength(before)

                vi.advanceTimersByTime(1)
                expect(MockWebSocket.instances).toHaveLength(before + 1)
            }
        })

        it('does not reconnect a socket closed by disconnect()', () => {
            const {disconnect} = useWebSocket('/ws/test', {
                reconnect: true,
                reconnectInterval: 100,
            })

            getWebSocket().simulateOpen()
            disconnect()
            // The browser fires onclose after close(); the mock has to be told to.
            MockWebSocket.instances[0].simulateClose()
            vi.advanceTimersByTime(5000)

            expect(MockWebSocket.instances).toHaveLength(1)
        })

        it('ignores a superseded socket closing after its replacement opened', async () => {
            const {connected, disconnect, connect} = useWebSocket('/ws/test', {
                reconnect: true,
                reconnectInterval: 100,
            })

            getWebSocket().simulateOpen()
            disconnect()
            connect()
            MockWebSocket.instances[1].simulateOpen()
            await nextTick()

            // The old socket's close event arrives late, as it does in a real browser.
            MockWebSocket.instances[0].simulateClose()
            await nextTick()
            vi.advanceTimersByTime(5000)

            expect(connected.value).toBe(true)
            expect(MockWebSocket.instances).toHaveLength(2)
        })

        it('resets reconnect attempts on successful connection', async () => {
            const {reconnectAttempts} = useWebSocket('/ws/test', {
                reconnect: true,
                reconnectInterval: 100,
            })

            getWebSocket().simulateClose()
            vi.advanceTimersByTime(100)
            expect(reconnectAttempts.value).toBe(1)

            MockWebSocket.instances[1].simulateOpen()
            await nextTick()
            expect(reconnectAttempts.value).toBe(0)
        })
    })

    describe('liveness', () => {
        const PING = WS_RECONNECT.pingInterval

        it('pings on every interval', async () => {
            useWebSocket('/ws/test')
            await openWebSocket()

            vi.advanceTimersByTime(PING)

            expect(MockWebSocket.lastSent).toBe('ping')
        })

        // A vanished peer can leave the socket OPEN for minutes, the page on stale state.
        it('drops a socket that answers no ping for two intervals, and reconnects', async () => {
            const onClose = vi.fn()
            const {connected} = useWebSocket('/ws/test', {onClose})
            await openWebSocket()

            vi.advanceTimersByTime(PING + 2 * PING - 1)
            expect(connected.value).toBe(true)
            vi.advanceTimersByTime(1)

            expect(connected.value).toBe(false)
            expect(getWebSocket().readyState).toBe(MockWebSocket.CLOSED)
            // The server logs a close with this code, naming the frame that was in flight.
            expect(getWebSocket().closedWith).toEqual({code: WS_CLOSE_UNRESPONSIVE, reason: 'No answer to ping'})
            expect(onClose).toHaveBeenCalledOnce()
            vi.advanceTimersByTime(WS_RECONNECT.interval)
            expect(MockWebSocket.instances).toHaveLength(2)
        })

        it('keeps a socket that answers, with a pong or anything else', async () => {
            const {connected} = useWebSocket('/ws/test', {onMessage: () => {}})
            await openWebSocket()

            for (const answer of ['pong', '{"type":"frame_captured"}', 'pong']) {
                vi.advanceTimersByTime(2 * PING)
                getWebSocket().simulateMessage(answer)
            }
            vi.advanceTimersByTime(2 * PING)

            expect(connected.value).toBe(true)
            expect(MockWebSocket.instances).toHaveLength(1)
        })

        it('waits out the liveness window it was given', async () => {
            const {connected} = useWebSocket('/ws/test', {livenessTimeout: 5 * PING})
            await openWebSocket()

            vi.advanceTimersByTime(PING + 5 * PING - 1)
            expect(connected.value).toBe(true)
            vi.advanceTimersByTime(1)

            expect(connected.value).toBe(false)
        })

        it('never drops a socket whose liveness window is null', async () => {
            const {connected} = useWebSocket('/ws/test', {livenessTimeout: null})
            await openWebSocket()

            vi.advanceTimersByTime(100 * PING)

            expect(connected.value).toBe(true)
            expect(MockWebSocket.lastSent).toBe('ping')
        })

        it('stops watching a socket closed by disconnect()', async () => {
            const {disconnect} = useWebSocket('/ws/test')
            await openWebSocket()
            vi.advanceTimersByTime(PING)

            disconnect()
            vi.advanceTimersByTime(10 * PING)

            expect(MockWebSocket.instances).toHaveLength(1)
        })
    })

    describe('disconnect', () => {
        it('disconnect() closes the websocket', async () => {
            const {disconnect, connected} = useWebSocket('/ws/test')

            await openWebSocket()
            expect(connected.value).toBe(true)

            disconnect()
            await nextTick()
            expect(connected.value).toBe(false)
        })
    })
})
