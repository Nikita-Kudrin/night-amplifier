import * as api from './api.js'
import {ApiError, ignoreProOnly, isProOnly, LONG_REQUEST_TIMEOUT_MS, REQUEST_TIMEOUT_MS} from './api.js'
import {envelope, refusal, stubFetch, textResponse} from '../test/http.js'

const JSON_HEADERS = {'Content-Type': 'application/json'}

/** What each call puts on the wire: `[name, args, url, method, body]`; no body sends no Content-Type. */
const CALLS = [
    ['getCapabilities', [], '/api/capabilities', 'GET'],
    ['getAiCompute', [], '/api/ai-compute', 'GET'],
    ['remeasureAiCompute', [], '/api/ai-compute/benchmark', 'POST'],
    ['startCapture', ['cam1'], '/api/capture/start', 'POST', {camera_id: 'cam1'}],
    ['startCapture', [], '/api/capture/start', 'POST', {}],
    // Roles travel on the body, and 'main' deliberately sends none of it on stop: that is
    // the exact request every release before roles existed made.
    ['startCapture', [null, 'guide'], '/api/capture/start', 'POST', {role: 'guide'}],
    ['stopCapture', [], '/api/capture/stop', 'POST'],
    ['stopCapture', ['guide'], '/api/capture/stop', 'POST', {role: 'guide'}],
    ['getSettings', [], '/api/settings', 'GET'],
    ['updateSettings', [{gain: 200, bin: 2}], '/api/settings', 'POST', {gain: 200, bin: 2}],
    ['getStackingTypes', [], '/api/settings/stacking-types', 'GET'],
    ['listCameras', [], '/api/cameras', 'GET'],
    ['listCameras', [{discover: false}], '/api/cameras?discover=false', 'GET'],
    ['dismissRecoveryFailure', ['guide'], '/api/cameras/recovery/guide', 'DELETE'],
    ['connectCamera', ['zwo_0'], '/api/cameras/zwo_0/connect', 'POST', {role: 'main'}],
    ['connectCamera', ['zwo_0', 'guide'], '/api/cameras/zwo_0/connect', 'POST', {role: 'guide'}],
    ['connectCamera', ['camera/with/slashes'], '/api/cameras/camera%2Fwith%2Fslashes/connect', 'POST', {role: 'main'}],
    ['disconnectCamera', ['zwo_0'], '/api/cameras/zwo_0/disconnect', 'POST'],
    ['disconnectCamera', ['zwo_0', {skipWarmup: true}], '/api/cameras/zwo_0/disconnect', 'POST', {skip_warmup: true}],
    ['setViewedCamera', ['guide'], '/api/view/camera', 'PUT', {camera: 'guide'}],
    ['configureSimulator', ['/data/subs'], '/api/simulator/configure', 'POST', {directory: '/data/subs'}],
    ['getSimulatorConfig', [], '/api/simulator/config', 'GET'],
    ['removeSimulatedCamera', [2], '/api/simulator/2', 'DELETE'],
    ['getPushToStatus', [], '/api/push-to/status', 'GET'],
    ['searchCatalog', ['M 42 & co'], '/api/push-to/catalog/search?query=M+42+%26+co&limit=20', 'GET'],
    ['searchCatalog', ['M31', 5], '/api/push-to/catalog/search?query=M31&limit=5', 'GET'],
    ['setTargetByName', ['M31'], '/api/push-to/target', 'POST', {name: 'M31'}],
    ['setTargetByCoordinates', [10.5, 41.2], '/api/push-to/target', 'POST', {ra_degrees: 10.5, dec_degrees: 41.2, name: null}],
    ['clearTarget', [], '/api/push-to/target', 'DELETE'],
    ['updatePushToConfig', [{fov_degrees: 1.2}], '/api/push-to/config', 'POST', {fov_degrees: 1.2}],
    ['cancelPushToSolve', [], '/api/push-to/cancel', 'POST'],
    ['getAstapStatus', [], '/api/astap/status', 'GET'],
    ['getAstapDatabases', [], '/api/astap/databases', 'GET'],
    ['installAstap', [], '/api/astap/install', 'POST', {database_types: ['D80']}],
    ['installAstap', [['G05']], '/api/astap/install', 'POST', {database_types: ['G05']}],
    ['getCatalogStatus', [], '/api/catalog/status', 'GET'],
    ['installCatalog', [], '/api/catalog/install', 'POST', {include_stars: false}],
    ['installCatalog', [true], '/api/catalog/install', 'POST', {include_stars: true}],
    ['getLicenseStatus', [], '/api/about/license', 'GET'],
    ['updateLicense', ['tok'], '/api/about/license', 'POST', {token: 'tok'}],
    ['getSoftwareLicenses', [], '/api/about/software-licenses', 'GET'],
]

describe('API Client', () => {
    let fetchMock

    beforeEach(() => {
        fetchMock = stubFetch()
    })

    describe.each(CALLS)('%s(%j)', (name, args, url, method, body) => {
        it(`sends ${method} ${url} and answers with the envelope's data`, async () => {
            fetchMock.mockResolvedValue(envelope({answer: name}))

            const result = await api[name](...args)

            const init = {method, cache: 'no-store', signal: expect.any(globalThis.AbortSignal)}
            if (body !== undefined) Object.assign(init, {headers: JSON_HEADERS, body: JSON.stringify(body)})
            expect(fetchMock).toHaveBeenCalledWith(url, init)
            expect(result).toEqual({answer: name})
        })
    })

    it('covers every endpoint', () => {
        const endpoints = Object.entries(api)
            .filter(([, value]) => value?.constructor?.name === 'AsyncFunction')
            .map(([name]) => name)
            .filter((name) => name !== 'fetchEyepieceSnapshot') // eyepieceSnapshot.test.js
        expect(new Set(CALLS.map(([name]) => name))).toEqual(new Set(endpoints))
    })

    describe('Error Handling', () => {
        it('rejects with the server message and status', async () => {
            fetchMock.mockResolvedValue(refusal('Push-To requires Night Amplifier Pro', 403))

            const error = await api.getPushToStatus().catch((e) => e)

            expect(error).toBeInstanceOf(ApiError)
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

            const error = await api.listCameras().catch((e) => e)

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

            const error = await api.getSettings().catch((e) => e)

            expect(error.message).toBe('Server returned an invalid response.')
            expect(error.status).toBe(200)
        })

        it('reads a gateway error page as an unreachable server, keeping the status', async () => {
            fetchMock.mockResolvedValue(textResponse('<html>Bad Gateway</html>', 502))

            const error = await api.getSettings().catch((e) => e)

            expect(error.message).toBe('Server unavailable. Please ensure the server is running.')
            expect(error.status).toBe(502)
        })

        // axum answers a body its `Json<UpdateSettingsRequest>` cannot read with 422 and a
        // plain-text reason; "ensure the server is running" sent the user the wrong way.
        it('reports a plain-text refusal in the server\'s words, with its status', async () => {
            const reason = 'Failed to deserialize the JSON body into the target type: eyepiece.screen_width: invalid type'
            fetchMock.mockResolvedValue(textResponse(`  ${reason}\n`, 422))

            const error = await api.updateSettings({eyepiece: {screen_width: ''}}).catch((e) => e)

            expect(error).toBeInstanceOf(ApiError)
            expect(error.status).toBe(422)
            expect(error.message).toBe(reason)
        })

        it('caps a long plain-text refusal', async () => {
            fetchMock.mockResolvedValue(textResponse('x'.repeat(5000), 400))

            const error = await api.getSettings().catch((e) => e)

            expect(error.message.length).toBeLessThan(400)
        })
    })

    describe('timeouts', () => {
        afterEach(() => vi.useRealTimers())

        /** A server that never answers: the request settles only when its signal aborts. */
        function hang() {
            fetchMock.mockImplementation((url, {signal}) => new Promise((resolve, reject) => {
                signal.addEventListener('abort', () => reject(new Error('aborted')))
            }))
        }

        // The settings store runs no read beside a write: one hung POST held every refresh.
        it('abandons a request the server does not answer in time', async () => {
            vi.useFakeTimers()
            hang()

            const settled = api.updateSettings({gain: 1}).catch((e) => e)
            await vi.advanceTimersByTimeAsync(REQUEST_TIMEOUT_MS)
            const error = await settled

            expect(error).toBeInstanceOf(ApiError)
            expect(error.status).toBe(0)
            expect(error.message).toMatch(/did not answer in time/)
        })

        it('gives a camera connect the long deadline', async () => {
            vi.useFakeTimers()
            hang()
            let error = null
            const settled = api.connectCamera('zwo_0').catch((e) => (error = e))

            await vi.advanceTimersByTimeAsync(REQUEST_TIMEOUT_MS)
            expect(error).toBeNull()
            await vi.advanceTimersByTimeAsync(LONG_REQUEST_TIMEOUT_MS - REQUEST_TIMEOUT_MS)
            await settled

            expect(error.message).toMatch(/did not answer in time/)
        })

        it('times out a body that stops arriving', async () => {
            vi.useFakeTimers()
            fetchMock.mockImplementation(async (url, {signal}) => ({
                ok: true,
                status: 200,
                text: () => new Promise((resolve, reject) => {
                    signal.addEventListener('abort', () => reject(new Error('aborted')))
                }),
            }))

            const settled = api.getSettings().catch((e) => e)
            await vi.advanceTimersByTimeAsync(REQUEST_TIMEOUT_MS)

            expect((await settled).message).toMatch(/did not answer in time/)
        })
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
