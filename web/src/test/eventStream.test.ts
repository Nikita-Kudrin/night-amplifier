import {mount} from '@vue/test-utils'
import {createEventStreamStub, serverEvent} from './eventStream'
import {SERVER_EVENT_EXAMPLES} from './api.examples'
import {SERVER_EVENTS} from '../composables/api.events'
import type {ServerEventType} from '../composables/api.events'
import {useEventStream} from '../composables/useEventStream'
import {MockWebSocket, setupGlobalWebSocketMock} from '../composables/__tests__/webSocketTestUtils'

vi.mock('../composables/api', async (importOriginal) =>
    (await import('./mockApi')).mockApi(await importOriginal()))

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
        serverEvent('capture_count')
        // @ts-expect-error 'frame_dropped' carries `dropped_count`, not `dropped`
        serverEvent('frame_dropped', {dropped: 3})
        // @ts-expect-error `role` is a `CameraRole`
        serverEvent('camera_reconnecting', {role: 'imaging'})
        expect(serverEvent('frame_dropped', {dropped_count: 3})).toMatchObject({type: 'frame_dropped', dropped_count: 3})
    })

    // A handler reads what the wire always carries without a fallback, so a test event must too.
    it('carries every field the event requires, the given ones over the example', () => {
        expect(serverEvent('capture_counts', {stacked_count: 7})).toEqual({
            ...SERVER_EVENT_EXAMPLES.capture_counts,
            stacked_count: 7,
        })
        expect(serverEvent('camera_reconnecting')).toEqual(expect.objectContaining({name: expect.any(String), role: 'main'}))
    })

    it('says nothing the test did not ask for: a nullable field null, a list empty, an optional one left out', () => {
        expect(serverEvent('capture_counts').last_rejection_reason).toBeNull()
        expect(serverEvent('camera_phases').cameras).toEqual([])
        expect(serverEvent('frame_captured')).not.toHaveProperty('rejection_reason')
    })

    it('takes a union event by the branch its fields name', () => {
        expect(serverEvent('astap_install_status', {state: 'failed', error: 'disk full'}))
            .toMatchObject({type: 'astap_install_status', state: 'failed', error: 'disk full'})
        // @ts-expect-error a `completed` status carries no `error`
        serverEvent('astap_install_status', {state: 'completed', error: 'x'})
    })

    it('hands out a fresh event each time', () => {
        const first = serverEvent('camera_phases')
        first.cameras.push({name: 'A', role: 'main', phase: 'idle'})
        expect(serverEvent('camera_phases').cameras).toEqual([])
    })
})
