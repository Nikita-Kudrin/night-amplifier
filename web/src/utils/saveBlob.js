/**
 * Hand fetched bytes to the browser as a file the user keeps.
 */

/**
 * How long the object URL stays alive after the click.
 *
 * Revoking it in the same tick can cancel the download: the browser has only
 * queued the save, not read the blob yet. Chrome commits on click, but Safari
 * (iOS included) reads it later, so this is seconds, not one event-loop turn —
 * the delay decides only when the URL is freed, not whether.
 */
export const URL_RELEASE_MS = 10000

/**
 * Save `blob` to the user's device under `filename`. `download` makes this a save
 * instead of a navigation: a `blob:` URL has no HTTP headers (`fetch` already
 * consumed the response), so `Content-Disposition` can't force it. Without it the
 * browser just navigates to and renders the blob, taking the page with it.
 * @param {Blob} blob - The bytes to save.
 * @param {string} filename - Name to save them under.
 */
export function saveBlob(blob, filename) {
    const url = URL.createObjectURL(blob)
    const link = document.createElement('a')
    link.href = url
    link.download = filename
    // Firefox only dispatches the click for a link that is in the document.
    document.body.appendChild(link)
    link.click()
    link.remove()
    setTimeout(() => URL.revokeObjectURL(url), URL_RELEASE_MS)
}
