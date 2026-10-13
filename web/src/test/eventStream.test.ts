import {mount} from '@vue/test-utils'
import {createEventStreamStub, serverEvent} from './eventStream.js'
import {SERVER_EVENTS} from '../composables/api.events.js'
import type {ServerEventType} from '../composables/api.events.js'
import {useEventStream} from '../composables/useEventStream.js'
import {MockWebSocket, setupGlobalWebSocketMock} from '../composables/__tests__/webSocketTestUtils.js'

vi.mock('../composables/api.js', async (importOriginal) =>
    (await import('./mockApi.js')).mockApi(await importOriginal()))

setupGlobalWebSocketMock()

describe('createEventStreamStub', () => {
    afterEach(() => {
        MockWebSocket.reset()
    })

    it('starts where the real one starts', () => {
        let real: ReturnType<typeof useEventStream> | undefined
        const wrapper = mount({
            setup() {
                real = useEventStream()
                return () => null
            },
        })
        const stub = createEventStreamStub()

        const keys = ['captureState', 'viewedCamera', 'stackCrop', 'plateSolving', 'currentTarget', 'droppedCount'] as const
        for (const key of keys) expect(stub[key].value, key).toEqual(real![key].value)
        wrapper.unmount()
    })

    it('takes overrides as plain values or refs, and clears what its actions clear', () => {
        const stub = createEventStreamStub({captureState: 'Capturing', lastError: 'Camera lost'})

        expect(stub.captureState.value).toBe('Capturing')
        stub.clearError()
        expect(stub.lastError.value).toBeNull()
        expect(stub.clearError).toHaveBeenCalledOnce()
    })
})

describe('serverEvent', () => {
    it('builds every event the server sends', () => {
        for (const type of Object.keys(SERVER_EVENTS) as ServerEventType[]) expect(serverEvent(type).type).toBe(type)
    })

    // Type-level: `tsconfig.vitest.json` is part of `npm run typecheck`.
    it('takes only the fields an event carries, with their wire types', () => {
        // @ts-expect-error the server sends no 'capture_count' event
        expect(() => serverEvent('capture_count')).toThrow("no 'capture_count' event")
        // @ts-expect-error 'frame_dropped' carries `dropped_count`, not `dropped`
        serverEvent('frame_dropped', {dropped: 3})
        // @ts-expect-error `role` is a `CameraRole`
        serverEvent('camera_reconnecting', {role: 'imaging'})
        expect(serverEvent('frame_dropped', {dropped_count: 3})).toEqual({type: 'frame_dropped', dropped_count: 3})
    })
})
