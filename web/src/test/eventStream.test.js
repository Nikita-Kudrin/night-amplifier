import {mount} from '@vue/test-utils'
import {createEventStreamStub} from './eventStream.js'
import {useEventStream} from '../composables/useEventStream.js'
import {MockWebSocket, setupGlobalWebSocketMock} from '../composables/__tests__/webSocketTestUtils.js'

vi.mock('../composables/api.js', async (importOriginal) =>
    (await import('./mockApi.js')).mockApi(await importOriginal()))

setupGlobalWebSocketMock()

describe('createEventStreamStub', () => {
    afterEach(() => {
        MockWebSocket.reset()
    })

    it('has every field and action the real event stream has', () => {
        let real
        const wrapper = mount({
            setup() {
                real = useEventStream()
                return () => null
            },
        })

        expect(Object.keys(createEventStreamStub()).sort()).toEqual(Object.keys(real).sort())
        wrapper.unmount()
    })

    it('starts where the real one starts', () => {
        let real
        const wrapper = mount({
            setup() {
                real = useEventStream()
                return () => null
            },
        })
        const stub = createEventStreamStub()

        for (const key of ['captureState', 'viewedCamera', 'stackCrop', 'plateSolving', 'currentTarget', 'droppedCount']) {
            expect(stub[key].value, key).toEqual(real[key].value)
        }
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
