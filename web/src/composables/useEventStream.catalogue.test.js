import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {effectScope, ref} from 'vue'
import {
    createTestContext,
    cleanupTestContext,
    openWebSocket,
    setupGlobalWebSocketMock,
    syncWebSocket,
} from './__tests__/webSocketTestUtils.js'

setupGlobalWebSocketMock()

vi.mock('./api.js', async (importOriginal) =>
    (await import('../test/mockApi.js')).mockApi(await importOriginal()))

import {getAiCompute, getSettings, listCameras} from './api.js'
import {SERVER_EVENTS, TOPICS} from './api.events.js'
import {useEventStream} from './useEventStream.js'
import {createAppState} from './useAppState.js'
import {createAiComputeStore} from './useAiCompute.js'

/** The REST read that restates each refetched topic; a new one must be added here. */
const REFETCHES = {settings: getSettings, ai_compute: getAiCompute, camera_list: listCameras}

function readCounts() {
    return Object.fromEntries(Object.entries(REFETCHES).map(([topic, read]) => [topic, read.mock.calls.length]))
}

/**
 * The real stream and stores, wired as `App.vue` wires them, held to the server's event
 * catalogue (`api.events.ts`): a state event nobody follows, or a handler for an event
 * the server never sends, fails here. Notices may go unheard.
 */
describe('the event handlers against the server catalogue', () => {
    let scope
    let stream

    beforeEach(async () => {
        createTestContext()
        vi.clearAllMocks()
        scope = effectScope()
        scope.run(() => {
            stream = useEventStream()
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

    it('follows every event that states or announces a topic', () => {
        const followed = new Set(stream.subscribedTypes())
        const unheard = Object.entries(SERVER_EVENTS)
            .filter(([type, {kind}]) => kind !== 'notice' && !followed.has(type))
            .map(([type, {topic}]) => `${type} (${topic})`)

        expect(unheard).toEqual([])
    })

    it('follows no event the server does not send', () => {
        expect(stream.subscribedTypes().filter((type) => !(type in SERVER_EVENTS))).toEqual([])
    })

    it('knows how to re-read every refetched topic', () => {
        const unmapped = Object.keys(TOPICS).filter((topic) => TOPICS[topic] === 'refetch' && !REFETCHES[topic])

        expect(unmapped, 'add the REST read of each to REFETCHES').toEqual([])
    })

    it('re-reads every refetched topic after a lag', async () => {
        await syncWebSocket()
        await vi.runOnlyPendingTimersAsync()
        const before = readCounts()

        await syncWebSocket({cause: 'lagged', dropped: 144})
        await vi.runOnlyPendingTimersAsync()

        const unread = Object.keys(TOPICS)
            .filter((topic) => TOPICS[topic] === 'refetch')
            .filter((topic) => !(readCounts()[topic] > before[topic]))
        expect(unread).toEqual([])
    })
})
