import {vi} from 'vitest'
import type {Mock} from 'vitest'

/**
 * Replace `fetch` for one test. Vitest's `unstubGlobals` puts the unmocked-fetch guard
 * back after it, where assigning `global.fetch` replaced the guard for the whole file.
 */
export function stubFetch(): Mock<typeof fetch> {
    const fetchMock = vi.fn<typeof fetch>()
    vi.stubGlobal('fetch', fetchMock)
    return fetchMock
}

/** A JSON route's answer in the server's `{success, data, error}` envelope. */
export function envelope(data: unknown, status = 200): Response {
    return textResponse(JSON.stringify({success: true, data}), status)
}

/** A refusal in the envelope, with the status the server sends it under. */
export function refusal(error: string, status = 400): Response {
    return textResponse(JSON.stringify({success: false, error}), status)
}

/** An answer with a body of `text`: only what `api.ts` reads of a JSON route's answer. */
export function textResponse(text: string, status = 200): Response {
    const response = {ok: status >= 200 && status < 300, status, text: async () => text}
    return response as unknown as Response
}

/** A request the test answers itself: `{promise, resolve, reject}`. */
export function pending<T>() {
    let resolve!: (value: T) => void
    let reject!: (error: unknown) => void
    const promise = new Promise<T>((res, rej) => {
        resolve = res
        reject = rej
    })
    return {promise, resolve, reject}
}
