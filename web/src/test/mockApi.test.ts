import * as actual from '../composables/api'
import {mockApi} from './mockApi'
import {astapStatus, settings} from './fixtures'

describe('mockApi', () => {
    it('mocks every endpoint and keeps the rest real', async () => {
        const api = mockApi(actual)

        expect(vi.isMockFunction(api.getSettings)).toBe(true)
        expect(vi.isMockFunction(api.fetchEyepieceSnapshot)).toBe(true)
        expect(api.ApiError).toBe(actual.ApiError)
        expect(api.isProOnly).toBe(actual.isProOnly)
        expect(api.REQUEST_TIMEOUT_MS).toBe(actual.REQUEST_TIMEOUT_MS)
        await expect(api.getSettings()).resolves.toEqual(settings())
    })

    it('resolves `{}` for a call without a default', async () => {
        await expect(mockApi(actual).clearTarget()).resolves.toEqual({})
    })

    it('takes an override over the default', async () => {
        const api = mockApi(actual, {getAstapStatus: vi.fn(async () => astapStatus({ready: false}))})

        await expect(api.getAstapStatus()).resolves.toEqual(astapStatus({ready: false}))
    })

    it('refuses an override of the wrong shape', () => {
        // @ts-expect-error -- not an `AstapStatus`
        mockApi(actual, {getAstapStatus: vi.fn(async () => ({ready: false}))})
        // @ts-expect-error -- not an endpoint
        mockApi(actual, {isProOnly: vi.fn()})
    })
})
