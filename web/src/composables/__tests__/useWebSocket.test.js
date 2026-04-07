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

import {useWebSocket} from '../useWebSocket.js'

describe('useWebSocket', () => {
    beforeEach(createTestContext)
    afterEach(cleanupTestContext)

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

        it('stops reconnecting after max attempts', () => {
            useWebSocket('/ws/test', {
                reconnect: true,
                reconnectInterval: 100,
                maxReconnectAttempts: 3,
            })

            for (let i = 0; i < 4; i++) {
                MockWebSocket.instances[i].simulateClose()
                vi.advanceTimersByTime(100)
            }

            expect(MockWebSocket.instances).toHaveLength(4)
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
