import {vi} from 'vitest'
import type {Mock} from 'vitest'
import type * as Api from '../composables/api.js'
import {
    aiComputeReport,
    astapStatus,
    capabilities,
    catalogStatus,
    pushToStatus,
    settings,
    stackingTypes,
} from './fixtures.js'

type ApiModule = typeof Api

/** The REST calls `api.ts` exports: every async function, as {@link isEndpoint} finds them. */
export type Endpoint = {
    [K in keyof ApiModule]: ApiModule[K] extends (...args: never[]) => Promise<unknown> ? K : never
}[keyof ApiModule]

/** Each endpoint as a `vi.fn()` of its own signature, so an override of the wrong shape fails. */
export type MockedApi = {[K in Endpoint]: Mock<ApiModule[K]>}

/** What `mockApi` stands in for `api.ts` with: the endpoints mocked, the rest real. */
export type MockedApiModule = Omit<ApiModule, Endpoint> & MockedApi

type Defaults = {[K in Endpoint]?: () => Awaited<ReturnType<ApiModule[K]>>}

/** Endpoints answer like an idle Community server with everything installed. */
const DEFAULTS: Defaults = {
    getSettings: () => settings(),
    updateSettings: () => settings(),
    getCapabilities: () => capabilities(),
    listCameras: () => [],
    getStackingTypes: () => stackingTypes(),
    getPushToStatus: () => pushToStatus(),
    searchCatalog: () => [],
    getAstapStatus: () => astapStatus(),
    getAstapDatabases: () => [],
    getCatalogStatus: () => catalogStatus(),
    getAiCompute: () => aiComputeReport(),
    getSimulatorConfig: () => ({configured: false}),
}

/**
 * A stand-in for `api.ts` with every endpoint a `vi.fn()`, so a file mocks the module
 * once instead of listing the calls its component happens to make today:
 *
 *   vi.mock('../composables/api.js', async (importOriginal) =>
 *       (await import('../test/mockApi.js')).mockApi(await importOriginal<typeof Api>(), {
 *           getSettings: vi.fn(async () => settings({gain: 100})),
 *       }))
 *
 * The non-endpoint exports (`ApiError`, `isProOnly`, the snapshot constants) stay real.
 * A call without a default resolves `{}`.
 */
export function mockApi(actual: ApiModule, overrides: Partial<MockedApi> = {}): MockedApiModule {
    const answers: Partial<Record<string, () => unknown>> = DEFAULTS
    const mocked: Record<string, unknown> = {...actual}
    for (const [name, value] of Object.entries(actual)) {
        if (!isEndpoint(value)) continue
        const answer = answers[name] ?? (() => ({}))
        mocked[name] = vi.fn(async () => answer())
    }
    // The one cast: the loop builds the endpoints by name, and `{}` is no endpoint's answer.
    return {...mocked, ...overrides} as MockedApiModule
}

/** The REST calls: every async function, which is all of them but the error helpers. */
function isEndpoint(value: unknown): boolean {
    return typeof value === 'function' && value.constructor.name === 'AsyncFunction'
}
