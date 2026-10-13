import {vi} from 'vitest'

/**
 * Replace `fetch` for one test. Vitest's `unstubGlobals` puts the unmocked-fetch guard
 * back after it, where assigning `global.fetch` replaced the guard for the whole file.
 * @returns {import('vitest').Mock}
 */
export function stubFetch() {
    const fetchMock = vi.fn()
    vi.stubGlobal('fetch', fetchMock)
    return fetchMock
}

/** A JSON route's answer in the server's `{success, data, error}` envelope. */
export function envelope(data, status = 200) {
    return textResponse(JSON.stringify({success: true, data}), status)
}

/** A refusal in the envelope, with the status the server sends it under. */
export function refusal(error, status = 400) {
    return textResponse(JSON.stringify({success: false, error}), status)
}

export function textResponse(text, status = 200) {
    return {ok: status >= 200 && status < 300, status, text: async () => text}
}
