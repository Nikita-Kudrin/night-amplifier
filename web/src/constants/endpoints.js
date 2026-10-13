/**
 * Where the server answers. REST calls go through `api.js` under `API_BASE`; the sockets
 * and the eyepiece pages are named here so a route is spelled once.
 */
export const API_BASE = '/api'

export const WS = Object.freeze({
    STREAM: '/ws/stream',
    EYEPIECE: '/ws/eyepiece',
    EYEPIECE_QUALITY: '/ws/eyepiece_quality',
    EVENTS: '/ws/events',
})

export const PAGES = Object.freeze({
    EYEPIECE: '/eyepiece',
    EYEPIECE_QUALITY: '/eyepiece_quality',
})

export function isEyepiecePage(path) {
    return path === PAGES.EYEPIECE || path === PAGES.EYEPIECE_QUALITY
}

/** `/`'s image socket for one camera; the source is fixed per socket. */
export function liveStreamEndpoint(source) {
    return source === 'guide' ? `${WS.STREAM}?source=guide` : WS.STREAM
}

/** The socket an eyepiece page shows: lossless on `/eyepiece_quality`, JPEG otherwise. */
export function eyepieceStreamEndpoint(path) {
    return path === PAGES.EYEPIECE_QUALITY ? WS.EYEPIECE_QUALITY : WS.EYEPIECE
}
