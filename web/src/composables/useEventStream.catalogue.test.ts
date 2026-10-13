import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {effectScope, ref} from 'vue'
import type {EffectScope} from 'vue'
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
import type {ServerEventType, Topic} from './api.events.js'
import {useEventStream} from './useEventStream.js'
import type {EventStream} from './useEventStream.js'
import {createAppState} from './useAppState.js'
import {createAiComputeStore} from './useAiCompute.js'
import type {createCaptureEvents} from './events/captureEvents.js'
import type {createPushToEvents} from './events/pushToEvents.js'
import type {createInstallEvents} from './events/installEvents.js'
import type {createSettingsStore} from './settingsStore.js'
import type {createCameraStore} from './cameraStore.js'

/** Every event that states or announces a topic; a notice may go unheard. */
type StateEventType = {
    [K in ServerEventType]: (typeof SERVER_EVENTS)[K]['kind'] extends 'notice' ? never : K
}[ServerEventType]

type RefetchTopic = {[T in Topic]: (typeof TOPICS)[T] extends 'refetch' ? T : never}[Topic]

/** What the handler tables follow: the stream's own domains, then the stores `App.vue` wires. */
type Followed =
    | keyof ReturnType<typeof createCaptureEvents>['handlers']
    | keyof ReturnType<typeof createPushToEvents>['handlers']
    | keyof ReturnType<typeof createInstallEvents>['handlers']
    | keyof ReturnType<typeof createSettingsStore>['handlers']
    | keyof ReturnType<typeof createCameraStore>['handlers']
    | keyof ReturnType<typeof createAiComputeStore>['handlers']

/** The REST read that restates each refetched topic; a new one does not compile until added here. */
const REFETCHES = {
    settings: getSettings,
    ai_compute: getAiCompute,
    camera_list: listCameras,
} satisfies Record<RefetchTopic, unknown>

function readCounts() {
    return Object.fromEntries(
        Object.entries(REFETCHES).map(([topic, read]) => [topic, vi.mocked(read).mock.calls.length]),
    )
}

/**
 * The real stream and stores, wired as `App.vue` wires them, held to the server's event
 * catalogue (`api.events.ts`). A handler for an event the server never sends does not
 * compile (`EventHandlers`), nor does a table that leaves a state event unheard (below).
 */
describe('the event handlers against the server catalogue', () => {
    let scope: EffectScope
    let stream: EventStream

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

    // Type-level: `tsconfig.vitest.json` is part of `npm run typecheck`.
    it('has a table entry for every event that states or announces a topic', () => {
        expectTypeOf<Exclude<StateEventType, Followed>>().toEqualTypeOf<never>()
    })

    // The tables are typed; this is what holds them attached.
    it('follows every event that states or announces a topic once wired', () => {
        const followed = new Set(stream.subscribedTypes())
        const unheard = (Object.keys(SERVER_EVENTS) as ServerEventType[])
            .filter((type) => SERVER_EVENTS[type].kind !== 'notice' && !followed.has(type))
            .map((type) => `${type} (${SERVER_EVENTS[type].topic})`)

        expect(unheard).toEqual([])
    })

    it('re-reads every refetched topic after a lag', async () => {
        await syncWebSocket()
        await vi.runOnlyPendingTimersAsync()
        const before = readCounts()

        await syncWebSocket({cause: 'lagged', dropped: 144})
        await vi.runOnlyPendingTimersAsync()

        const unread = (Object.keys(TOPICS) as Topic[])
            .filter((topic) => TOPICS[topic] === 'refetch')
            .filter((topic) => !(readCounts()[topic] > before[topic]))
        expect(unread).toEqual([])
    })
})
