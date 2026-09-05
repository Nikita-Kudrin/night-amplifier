/**
 * Hand fetched bytes to the browser as a file the user keeps.
 */

/**
 * How long the object URL stays alive after the click.
 *
 * Revoking it in the same tick can cancel the download it was created for: the
 * browser has only queued the save at that point, not read the blob. Chrome
 * commits on the click, but Safari — iOS included — reads it later, so this is
 * seconds rather than one turn of the event loop. The delay only decides when the
 * URL is freed, not whether.
 */
export const URL_RELEASE_MS = 10000

/**
 * Save `blob` to the user's device under `filename`.
 *
 * The `download` attribute is what makes this a save instead of a navigation, and
 * it is not optional on a `blob:` URL: such a URL carries no HTTP headers, because
 * `fetch` has already consumed the response, so `Content-Disposition` can neither
 * name the file nor force the save the way it does on a network URL. Without the
 * attribute the browser just navigates to the blob and renders it in the tab —
 * taking the page, and any stream it was holding open, with it.
 *
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
