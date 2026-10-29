import * as api from './api'
import {
    ApiError,
    ignoreProOnly,
    isApiError,
    isProOnly,
    LONG_REQUEST_TIMEOUT_MS,
    REQUEST_TIMEOUT_MS,
} from './api'
import {envelope, refusal, stubFetch, textResponse} from '../test/http'
import type {Endpoint} from '../test/mockApi'
import type {Mock} from 'vitest'

const JSON_HEADERS = {'Content-Type': 'application/json'}

type Api = typeof api
type Call = [
    name: Endpoint,
    args: unknown[],
    url: string,
    method: string,
    body: unknown,
    send: () => Promise<unknown>,
]

/** What a call puts on the wire; no body sends no Content-Type. `args` are held to its signature. */
function call<K extends Endpoint>(
    name: K,
    args: Parameters<Api[K]>,
    url: string,
    method: string,
    body?: unknown
): Call {
    const endpoint = api[name] as (...args: Parameters<Api[K]>) => Promise<unknown>
    return [name, args, url, method, body, () => endpoint(...args)]
}

const CALLS: Call[] = [
    call('getCapabilities', [], '/api/capabilities', 'GET'),
    call('getAiCompute', [], '/api/ai-compute', 'GET'),
    call('remeasureAiCompute', [], '/api/ai-compute/benchmark', 'POST'),
    call('startCapture', ['cam1'], '/api/capture/start', 'POST', {camera_id: 'cam1'}),
    call('startCapture', [], '/api/capture/start', 'POST', {}),
    // Roles travel on the body, and 'main' deliberately sends none of it on stop: that is
    // the exact request every release before roles existed made.
    call('startCapture', [null, 'guide'], '/api/capture/start', 'POST', {role: 'guide'}),
    call('stopCapture', [], '/api/capture/stop', 'POST'),
    call('stopCapture', ['guide'], '/api/capture/stop', 'POST', {role: 'guide'}),
    call('getSettings', [], '/api/settings', 'GET'),
    call('updateSettings', [{gain: 200, bin: 2}], '/api/settings', 'POST', {gain: 200, bin: 2}),
    call('getStackingTypes', [], '/api/settings/stacking-types', 'GET'),
    call('listCameras', [], '/api/cameras', 'GET'),
    call('listCameras', [{discover: false}], '/api/cameras?discover=false', 'GET'),
    call('dismissRecoveryFailure', ['guide'], '/api/cameras/recovery/guide', 'DELETE'),
    call('calibrateDaylightBalance', ['guide'], '/api/cameras/daylight-balance/guide/calibrate', 'POST'),
    call('connectCamera', ['zwo_0'], '/api/cameras/zwo_0/connect', 'POST', {role: 'main'}),
    call('connectCamera', ['zwo_0', 'guide'], '/api/cameras/zwo_0/connect', 'POST', {role: 'guide'}),
    call('connectCamera', ['camera/with/slashes'], '/api/cameras/camera%2Fwith%2Fslashes/connect', 'POST', {role: 'main'}),
    call('disconnectCamera', ['zwo_0'], '/api/cameras/zwo_0/disconnect', 'POST'),
    call('disconnectCamera', ['zwo_0', {skipWarmup: true}], '/api/cameras/zwo_0/disconnect', 'POST', {skip_warmup: true}),
    call('setViewedCamera', ['guide'], '/api/view/camera', 'PUT', {camera: 'guide'}),
    call('configureSimulator', ['/data/subs'], '/api/simulator/configure', 'POST', {directory: '/data/subs'}),
    call('getSimulatorConfig', [], '/api/simulator/config', 'GET'),
    call('removeSimulatedCamera', [2], '/api/simulator/2', 'DELETE'),
    call('searchCatalog', ['M 42 & co'], '/api/push-to/catalog/search?query=M+42+%26+co&limit=20', 'GET'),
    call('searchCatalog', ['M31', 5], '/api/push-to/catalog/search?query=M31&limit=5', 'GET'),
    call('setTargetByName', ['M31'], '/api/push-to/target', 'POST', {name: 'M31'}),
    // The server tries a name first and would aim at it over the coordinates, so none is sent,
    // and passing one does not compile.
    call('setTargetByCoordinates', [10.5, 41.2], '/api/push-to/target', 'POST', {ra_degrees: 10.5, dec_degrees: 41.2}),
    // @ts-expect-error -- no name parameter
    call('setTargetByCoordinates', [10.5, 41.2, 'M31'], '/api/push-to/target', 'POST', {ra_degrees: 10.5, dec_degrees: 41.2}),
    call('clearTarget', [], '/api/push-to/target', 'DELETE'),
    call('updatePushToConfig', [{fov_degrees: 1.2}], '/api/push-to/config', 'POST', {fov_degrees: 1.2}),
    call('cancelPushToSolve', [], '/api/push-to/cancel', 'POST'),
    call('getAstapStatus', [], '/api/astap/status', 'GET'),
    call('getAstapDatabases', [], '/api/astap/databases', 'GET'),
    call('installAstap', [], '/api/astap/install', 'POST', {database_types: ['D80']}),
    call('installAstap', [['G05']], '/api/astap/install', 'POST', {database_types: ['G05']}),
    call('getCatalogStatus', [], '/api/catalog/status', 'GET'),
    call('installCatalog', [], '/api/catalog/install', 'POST', {include_stars: false}),
    call('installCatalog', [true], '/api/catalog/install', 'POST', {include_stars: true}),
    call('getLicenseStatus', [], '/api/about/license', 'GET'),
    call('updateLicense', ['tok'], '/api/about/license', 'POST', {token: 'tok'}),
    call('getSoftwareLicenses', [], '/api/about/software-licenses', 'GET'),
]

/** The `ApiError` a call rejects with; anything else fails the test. */
async function apiError(pending: Promise<unknown>): Promise<ApiError> {
    const error = await pending.then(() => 'resolved', (e: unknown) => e)
    if (!isApiError(error)) throw new Error(`expected an ApiError, got ${String(error)}`)
    return error
}

describe('API Client', () => {
    let fetchMock: Mock<typeof fetch>

    beforeEach(() => {
        fetchMock = stubFetch()
    })

    describe.each(CALLS)('%s(%j)', (name, _args, url, method, body, send) => {
        it(`sends ${method} ${url} and answers with the envelope's data`, async () => {
            fetchMock.mockResolvedValue(envelope({answer: name}))

            const result = await send()

            const init: RequestInit = {method, cache: 'no-store', signal: expect.any(globalThis.AbortSignal)}
            if (body !== undefined) Object.assign(init, {headers: JSON_HEADERS, body: JSON.stringify(body)})
            expect(fetchMock).toHaveBeenCalledWith(url, init)
            expect(result).toEqual({answer: name})
        })
    })

    it('covers every endpoint', () => {
        const endpoints = Object.entries(api)
            .filter(([, value]) => value?.constructor?.name === 'AsyncFunction')
            .map(([name]) => name)
            .filter((name) => name !== 'fetchEyepieceSnapshot') // eyepieceSnapshot.test.ts
        expect(new Set(CALLS.map(([name]) => name))).toEqual(new Set(endpoints))
    })

    describe('Error Handling', () => {
        it('rejects with the server message and status', async () => {
            fetchMock.mockResolvedValue(refusal('Push-To requires Night Amplifier Pro', 403))

            const error = await apiError(api.getAstapStatus())

            expect(error.message).toBe('Push-To requires Night Amplifier Pro')
            expect(error.status).toBe(403)
            expect(isProOnly(error)).toBe(true)
        })

        it('throws generic error when no error message provided', async () => {
            fetchMock.mockResolvedValue(textResponse(JSON.stringify({success: false}), 500))

            await expect(api.getSettings()).rejects.toThrow('API request failed')
        })

        it('throws user-friendly error on network failure, with no status', async () => {
            fetchMock.mockRejectedValue(new TypeError('Network error'))

            const error = await apiError(api.listCameras())

            expect(error.message).toBe('Server unavailable. Please ensure the server is running.')
            expect(error.status).toBe(0)
            expect(isProOnly(error)).toBe(false)
        })

        it('throws error on empty response', async () => {
            fetchMock.mockResolvedValue(textResponse('', 502))

            await expect(api.getSettings()).rejects.toThrow(
                'Server unavailable. Please ensure the server is running.'
            )
        })

        it('reports a non-JSON 200 as an invalid answer, keeping the status', async () => {
            fetchMock.mockResolvedValue(textResponse('<html>index</html>', 200))

            const error = await apiError(api.getSettings())

            expect(error.message).toBe('Server returned an invalid response.')
            expect(error.status).toBe(200)
        })

        it('reads a gateway error page as an unreachable server, keeping the status', async () => {
            fetchMock.mockResolvedValue(textResponse('<html>Bad Gateway</html>', 502))

            const error = await apiError(api.getSettings())

            expect(error.message).toBe('Server unavailable. Please ensure the server is running.')
            expect(error.status).toBe(502)
        })

        // axum answers a body its `Json<UpdateSettingsRequest>` cannot read with 422 and a
        // plain-text reason; "ensure the server is running" sent the user the wrong way.
        it('reports a plain-text refusal in the server\'s words, with its status', async () => {
            const reason = 'Failed to deserialize the JSON body into the target type: eyepiece.screen_width: invalid type'
            fetchMock.mockResolvedValue(textResponse(`  ${reason}\n`, 422))

            // @ts-expect-error -- an emptied number field is no number, which the type now refuses too
            const error = await apiError(api.updateSettings({eyepiece: {screen_width: ''}}))

            expect(error.status).toBe(422)
            expect(error.message).toBe(reason)
        })

        it('caps a long plain-text refusal', async () => {
            fetchMock.mockResolvedValue(textResponse('x'.repeat(5000), 400))

            const error = await apiError(api.getSettings())

            expect(error.message.length).toBeLessThan(400)
        })
    })

    describe('timeouts', () => {
        afterEach(() => vi.useRealTimers())

        /** Rejects once `init`'s signal aborts, and never otherwise. */
        const untilAborted = (init: RequestInit | undefined) => new Promise<never>((_resolve, reject) => {
            init?.signal?.addEventListener('abort', () => reject(new Error('aborted')))
        })

        /** A server that never answers: the request settles only when its signal aborts. */
        function hang() {
            fetchMock.mockImplementation((_url, init) => untilAborted(init))
        }

        // The settings store runs no read beside a write: one hung POST held every refresh.
        it('abandons a request the server does not answer in time', async () => {
            vi.useFakeTimers()
            hang()

            const settled = apiError(api.updateSettings({gain: 1}))
            await vi.advanceTimersByTimeAsync(REQUEST_TIMEOUT_MS)
            const error = await settled

            expect(error.status).toBe(0)
            expect(error.message).toMatch(/did not answer in time/)
        })

        it('gives a camera connect the long deadline', async () => {
            vi.useFakeTimers()
            hang()
            let error: ApiError | null = null
            const settled = apiError(api.connectCamera('zwo_0')).then((e) => (error = e))

            await vi.advanceTimersByTimeAsync(REQUEST_TIMEOUT_MS)
            expect(error).toBeNull()
            await vi.advanceTimersByTimeAsync(LONG_REQUEST_TIMEOUT_MS - REQUEST_TIMEOUT_MS)

            expect((await settled).message).toMatch(/did not answer in time/)
        })

        it('times out a body that stops arriving', async () => {
            vi.useFakeTimers()
            // A platform fake: only what `request()` reads of a response.
            fetchMock.mockImplementation(async (_url, init) => ({
                ok: true,
                status: 200,
                text: () => untilAborted(init),
            }) as unknown as Response)

            const settled = apiError(api.getSettings())
            await vi.advanceTimersByTimeAsync(REQUEST_TIMEOUT_MS)

            expect((await settled).message).toMatch(/did not answer in time/)
        })
    })
})

describe('isApiError', () => {
    it('tells an ApiError from any other failure', () => {
        expect(isApiError(new ApiError('Pro only', 403))).toBe(true)
        expect(isApiError(new Error('Pro only'))).toBe(false)
        expect(isApiError({message: 'Pro only', status: 403})).toBe(false)
        expect(isProOnly({message: 'Pro only', status: 403})).toBe(false)
    })
})

describe('ignoreProOnly', () => {
    afterEach(() => {
        vi.restoreAllMocks()
    })

    it("swallows a Community server's 403 quietly", () => {
        const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
        ignoreProOnly(new ApiError('Pro only', 403), 'Push-To status')
        expect(warn).not.toHaveBeenCalled()
    })

    it('logs any other failure rather than losing it', () => {
        const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
        ignoreProOnly(new ApiError('Server unavailable'), 'Push-To status')
        expect(warn).toHaveBeenCalledWith('Push-To status failed:', 'Server unavailable')
    })
})
