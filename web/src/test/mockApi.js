import {vi} from 'vitest'
import {astapStatus, capabilities, catalogStatus, pushToStatus, settings} from './fixtures.js'

/**
 * A stand-in for `api.js` with every endpoint a `vi.fn()`, so a file mocks the module
 * once instead of listing the calls its component happens to make today:
 *
 *   vi.mock('../composables/api.js', async (importOriginal) =>
 *       (await import('../test/mockApi.js')).mockApi(await importOriginal(), {
 *           getSettings: vi.fn().mockResolvedValue(settings({gain: 100})),
 *       }))
 *
 * The non-endpoint exports (`ApiError`, `isProOnly`, the snapshot constants) stay real.
 * Endpoints answer like an idle Community server with everything installed; a call
 * without a default resolves `{}`.
 */
export function mockApi(actual, overrides = {}) {
    const defaults = {
        getSettings: () => settings(),
        updateSettings: () => settings(),
        getCapabilities: () => capabilities(),
        listCameras: () => [],
        getStackingTypes: () => [],
        getPushToStatus: () => pushToStatus(),
        searchCatalog: () => [],
        getAstapStatus: () => astapStatus(),
        getAstapDatabases: () => [],
        getCatalogStatus: () => catalogStatus(),
        getAiCompute: () => ({state: 'unavailable', rungs: []}),
        getSimulatorConfig: () => ({configured: false}),
    }
    const mocked = {...actual}
    for (const [name, value] of Object.entries(actual)) {
        if (!isEndpoint(value)) continue
        const answer = defaults[name] ?? (() => ({}))
        mocked[name] = vi.fn(async () => answer())
    }
    return {...mocked, ...overrides}
}

/** The REST calls: every async function, which is all of them but the error helpers. */
function isEndpoint(value) {
    return typeof value === 'function' && value.constructor.name === 'AsyncFunction'
}
