/**
 * Fails any test whose code reaches the network. happy-dom resolves `/api/...` against
 * `http://localhost:3000`, so an unmocked `api.js` call went out for real: refused and
 * swallowed by the caller's `catch` (40 stacks a run from `useEventStream`'s `onOpen`),
 * or, with anything listening on that port, answered mid-test.
 *
 * The call is refused like a network error, so the code under test takes its usual error
 * path, and the test fails afterwards naming the URL, even where that path ignores it. A
 * file stubbing `fetch` itself replaces this for that file only.
 */
import {afterAll, afterEach} from 'vitest'

let unmocked = []

globalThis.fetch = (input) => {
    const url = String(input?.url ?? input)
    unmocked.push(url)
    return Promise.reject(new TypeError(`unmocked fetch: ${url}`))
}

// A call landing after its test ended is reported by the next test: still named, not lost.
function reportUnmocked() {
    if (!unmocked.length) return
    const urls = unmocked
    unmocked = []
    throw new Error(`unmocked fetch (mock api.js or stub fetch): ${urls.join(', ')}`)
}

afterEach(reportUnmocked)
afterAll(reportUnmocked)
