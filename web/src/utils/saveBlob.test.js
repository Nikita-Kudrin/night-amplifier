import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {saveBlob, URL_RELEASE_MS} from './saveBlob.js'

describe('saveBlob', () => {
    let links

    beforeEach(() => {
        links = []
        global.URL.createObjectURL = vi.fn(() => 'blob:saved')
        global.URL.revokeObjectURL = vi.fn()
        // Clicking a real anchor in jsdom navigates; record the element instead.
        const realCreate = document.createElement.bind(document)
        vi.spyOn(document, 'createElement').mockImplementation((tag) => {
            const el = realCreate(tag)
            if (tag === 'a') {
                el.click = () => links.push({el, inDocument: el.isConnected})
            }
            return el
        })
    })

    afterEach(() => {
        vi.restoreAllMocks()
    })

    const png = () => new Blob(['png'], {type: 'image/png'})

    /**
     * The whole point of the helper. A `blob:` URL has no `Content-Disposition` to
     * force a save, so without this attribute the browser navigates to the blob and
     * renders it in the tab rather than writing a file.
     */
    it('marks the link as a download', () => {
        saveBlob(png(), 'eyepiece_05-09-2026_21-14-03.png')

        expect(links).toHaveLength(1)
        expect(links[0].el.getAttribute('download')).toBe('eyepiece_05-09-2026_21-14-03.png')
    })

    it('points the link at the blob', () => {
        saveBlob(png(), 'eyepiece.png')

        expect(global.URL.createObjectURL).toHaveBeenCalledTimes(1)
        expect(links[0].el.href).toContain('blob:saved')
    })

    // Firefox only dispatches the click for a link that is in the document.
    it('clicks the link while it is in the document', () => {
        saveBlob(png(), 'eyepiece.png')

        expect(links[0].inDocument).toBe(true)
    })

    it('leaves no link behind', () => {
        saveBlob(png(), 'eyepiece.png')

        expect(document.querySelector('a[download]')).toBeNull()
    })

    // Revoking in the same tick can cancel the download the URL was made for:
    // the browser has queued the save at that point, not read the blob.
    it('keeps the object URL alive until the browser has read it', () => {
        vi.useFakeTimers()

        saveBlob(png(), 'eyepiece.png')
        vi.advanceTimersByTime(URL_RELEASE_MS - 1)
        expect(global.URL.revokeObjectURL).not.toHaveBeenCalled()

        vi.advanceTimersByTime(1)

        expect(global.URL.revokeObjectURL).toHaveBeenCalledWith('blob:saved')
        vi.useRealTimers()
    })

    // Safari reads the blob well after the click, so a one-tick release is not
    // enough for it — the delay has to outlive the gap by a wide margin.
    it('waits seconds rather than a tick before releasing', () => {
        expect(URL_RELEASE_MS).toBeGreaterThanOrEqual(1000)
    })
})
