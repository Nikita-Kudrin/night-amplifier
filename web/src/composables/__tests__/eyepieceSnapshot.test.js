import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {
    fetchEyepieceSnapshot,
    SNAPSHOT_RETRY_MS,
    SNAPSHOT_RETRY_WINDOW_MS,
} from '../api.js'

const SERVER_FILENAME = 'eyepiece_05-09-2026_21-14-03.png'

function pngResponse(disposition = `attachment; filename="${SERVER_FILENAME}"`) {
    return {
        ok: true,
        status: 200,
        blob: async () => new Blob(['png'], {type: 'image/png'}),
        headers: {get: () => disposition},
    }
}

/** The server's "one snapshot at a time" refusal. */
function busyResponse() {
    return {ok: false, status: 503}
}

/**
 * Await a rejection that only happens once the fake clock moves.
 *
 * The assertion has to subscribe *before* the advance: the promise rejects while
 * the timers run, so a `rejects` assertion written afterwards attaches to an
 * already-rejected promise. The test still passes, but Node has reported an
 * unhandled rejection by then and vitest counts it against the run.
 */
async function rejectsWhile(pending, message, advance) {
    const rejected = expect(pending).rejects.toThrow(message)
    await advance()
    await rejected
}

describe('fetchEyepieceSnapshot', () => {
    beforeEach(() => {
        global.fetch = vi.fn()
    })

    afterEach(() => {
        vi.restoreAllMocks()
    })

    it('asks for the circular shape', async () => {
        global.fetch.mockResolvedValue(pngResponse())

        await fetchEyepieceSnapshot(true)

        expect(global.fetch).toHaveBeenCalledWith(
            '/api/eyepiece/snapshot?circular=true',
            expect.objectContaining({cache: 'no-store'})
        )
    })

    it('asks for the uncropped shape', async () => {
        global.fetch.mockResolvedValue(pngResponse())

        await fetchEyepieceSnapshot(false)

        expect(global.fetch).toHaveBeenCalledWith(
            '/api/eyepiece/snapshot?circular=false',
            expect.anything()
        )
    })

    // The body is a PNG, so it must not go anywhere near the JSON `request()` path.
    it('returns the response body as a blob', async () => {
        global.fetch.mockResolvedValue(pngResponse())

        const {blob} = await fetchEyepieceSnapshot(true)

        expect(blob).toBeInstanceOf(Blob)
        expect(blob.type).toBe('image/png')
    })

    /**
     * The name has to come back with the bytes. The server stamps the timestamp into
     * it, and the blob the caller ends up saving carries none of the headers it
     * arrived with — so a name not lifted off here is a name lost.
     */
    it('returns the name the server put in Content-Disposition', async () => {
        global.fetch.mockResolvedValue(pngResponse())

        const {filename} = await fetchEyepieceSnapshot(true)

        expect(filename).toBe(SERVER_FILENAME)
    })

    it('falls back to a name of its own when the header is missing', async () => {
        global.fetch.mockResolvedValue(pngResponse(null))

        const {filename} = await fetchEyepieceSnapshot(true)

        expect(filename).toBe('eyepiece.png')
    })

    it('falls back when the header carries no filename', async () => {
        global.fetch.mockResolvedValue(pngResponse('attachment'))

        const {filename} = await fetchEyepieceSnapshot(true)

        expect(filename).toBe('eyepiece.png')
    })

    // 404, not 503: nothing has been rendered yet, and retrying cannot change that
    // until a capture produces a frame.
    it('reports an empty stream without retrying', async () => {
        global.fetch.mockResolvedValue({ok: false, status: 404})

        await expect(fetchEyepieceSnapshot(true)).rejects.toThrow('No frame to download yet.')
        expect(global.fetch).toHaveBeenCalledTimes(1)
    })

    it('reports the status for any other failure', async () => {
        global.fetch.mockResolvedValue({ok: false, status: 500})

        await expect(fetchEyepieceSnapshot(true)).rejects.toThrow('Download failed (500).')
    })

    it('reports an unreachable server', async () => {
        global.fetch.mockRejectedValue(new TypeError('Failed to fetch'))

        await expect(fetchEyepieceSnapshot(true)).rejects.toThrow('Server unavailable')
    })

    /**
     * A busy server is rendering somebody else's snapshot, which is worth waiting
     * out: the render is bounded, and the alternative is telling an observer with
     * one frame on screen to click again themselves.
     */
    describe('retrying a busy server', () => {
        beforeEach(() => vi.useFakeTimers())
        afterEach(() => vi.useRealTimers())

        it('waits and asks again', async () => {
            global.fetch
                .mockResolvedValueOnce(busyResponse())
                .mockResolvedValueOnce(pngResponse())
            const pending = fetchEyepieceSnapshot(true)

            await vi.advanceTimersByTimeAsync(SNAPSHOT_RETRY_MS)

            await expect(pending).resolves.toMatchObject({filename: SERVER_FILENAME})
            expect(global.fetch).toHaveBeenCalledTimes(2)
        })

        it('does not ask again before the retry interval is up', async () => {
            global.fetch.mockResolvedValue(busyResponse())
            const pending = fetchEyepieceSnapshot(true).catch(() => {})

            await vi.advanceTimersByTimeAsync(SNAPSHOT_RETRY_MS - 1)

            expect(global.fetch).toHaveBeenCalledTimes(1)
            await vi.advanceTimersByTimeAsync(SNAPSHOT_RETRY_WINDOW_MS)
            await pending
        })

        it('keeps trying across the whole window', async () => {
            global.fetch.mockResolvedValue(busyResponse())
            const pending = fetchEyepieceSnapshot(true).catch((e) => e)

            await vi.advanceTimersByTimeAsync(SNAPSHOT_RETRY_WINDOW_MS)
            await pending

            // One attempt up front, then one per interval that fits in the window.
            const expected = Math.ceil(SNAPSHOT_RETRY_WINDOW_MS / SNAPSHOT_RETRY_MS)
            expect(global.fetch.mock.calls.length).toBeGreaterThanOrEqual(expected - 1)
            expect(global.fetch.mock.calls.length).toBeLessThanOrEqual(expected + 1)
        })

        it('gives up once the window closes', async () => {
            global.fetch.mockResolvedValue(busyResponse())

            await rejectsWhile(fetchEyepieceSnapshot(true), 'Server busy', () =>
                vi.advanceTimersByTimeAsync(SNAPSHOT_RETRY_WINDOW_MS)
            )
        })

        // The window is a deadline, not a retry count: a slow response eats into it
        // rather than extending it.
        it('stops waiting even when each attempt is slow', async () => {
            global.fetch.mockImplementation(
                () => new Promise((resolve) => setTimeout(() => resolve(busyResponse()), 1500))
            )
            const pending = fetchEyepieceSnapshot(true).catch((e) => e)

            await vi.advanceTimersByTimeAsync(SNAPSHOT_RETRY_WINDOW_MS * 2)
            const result = await pending

            expect(result).toBeInstanceOf(Error)
            expect(result.message).toContain('Server busy')
        })

        it('succeeds on a late attempt inside the window', async () => {
            global.fetch
                .mockResolvedValueOnce(busyResponse())
                .mockResolvedValueOnce(busyResponse())
                .mockResolvedValueOnce(busyResponse())
                .mockResolvedValue(pngResponse())
            const pending = fetchEyepieceSnapshot(true)

            await vi.advanceTimersByTimeAsync(SNAPSHOT_RETRY_MS * 3)

            await expect(pending).resolves.toMatchObject({filename: SERVER_FILENAME})
        })

        // A 404 arriving mid-retry is still terminal.
        it('stops retrying if the stream goes empty', async () => {
            global.fetch
                .mockResolvedValueOnce(busyResponse())
                .mockResolvedValue({ok: false, status: 404})

            await rejectsWhile(fetchEyepieceSnapshot(true), 'No frame to download yet.', () =>
                vi.advanceTimersByTimeAsync(SNAPSHOT_RETRY_MS)
            )

            expect(global.fetch).toHaveBeenCalledTimes(2)
        })
    })
})
