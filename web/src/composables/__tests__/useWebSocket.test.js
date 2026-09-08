import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {nextTick} from 'vue'
import {
    MockWebSocket,
    createTestContext,
    cleanupTestContext,
    getWebSocket,
    openWebSocket,
    setupGlobalWebSocketMock,
} from './webSocketTestUtils.js'

setupGlobalWebSocketMock()

import {useWebSocket as originalUseWebSocket} from '../useWebSocket.js'
import { mount } from '@vue/test-utils'

let currentApp = null;
function useWebSocket(...args) {
    let result;
    currentApp = mount({
        setup() {
            result = originalUseWebSocket(...args)
            return () => {}
        }
    })
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
        ])('calls %s callback on %s', async (callbackName, simulateMethod) => {
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
