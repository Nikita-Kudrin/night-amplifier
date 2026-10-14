import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {effectScope, ref} from 'vue'
import type {EffectScope} from 'vue'
import {
    createTestContext,
    cleanupTestContext,
    openWebSocket,
    setupGlobalWebSocketMock,
    syncWebSocket,
} from './webSocketTestUtils'

setupGlobalWebSocketMock()

vi.mock('../api', async (importOriginal) =>
    (await import('../../test/mockApi')).mockApi(await importOriginal()))

import {getAiCompute, getPushToStatus, getSettings, listCameras} from '../api'
import {useEventStream} from '../useEventStream'
import {createAppState} from '../useAppState'
import {createAiComputeStore} from '../useAiCompute'

/**
 * Every startup read the page makes, and the stores following one real event stream. Push-To
 * is never read: its state travels in the snapshot.
 */
describe('a sync of the real event stream', () => {
    const reads = {getSettings, listCameras, getAiCompute, getPushToStatus}
    let scope: EffectScope

    beforeEach(async () => {
        createTestContext()
        vi.clearAllMocks()
        scope = effectScope()
        scope.run(() => {
            const stream = useEventStream()
            const app = createAppState()
            const aiCompute = createAiComputeStore()
            app.connectEvents(stream)
            aiCompute.connectEvents({stream, settings: ref(null)})
            app.initializeState()
            aiCompute.refresh()
        })
        await openWebSocket()
    })

    afterEach(() => {
        scope.stop()
        cleanupTestContext()
    })

    function callCounts() {
        return Object.fromEntries(Object.entries(reads).map(([name, read]) => [name, vi.mocked(read).mock.calls.length]))
    }

    it('reads nothing the startup load is reading already on the first sync', async () => {
        await syncWebSocket()
        await vi.runOnlyPendingTimersAsync()

        expect(callCounts()).toEqual({getSettings: 1, listCameras: 1, getAiCompute: 1, getPushToStatus: 0})
    })

    it('reads everything no event restates again after a lag', async () => {
        await syncWebSocket()
        await vi.runOnlyPendingTimersAsync()

        await syncWebSocket({cause: 'lagged', dropped: 144})
        await vi.runOnlyPendingTimersAsync()

        expect(callCounts()).toEqual({getSettings: 2, listCameras: 2, getAiCompute: 2, getPushToStatus: 0})
        // A sync follows every reconnect and lag: enumerating the USB bus each time opened
        // every idle QHY/ZWO camera for seconds, mid-capture too.
        expect(listCameras).toHaveBeenLastCalledWith({discover: false})
    })
})
