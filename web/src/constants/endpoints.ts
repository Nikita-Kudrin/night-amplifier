/**
 * Where the server answers. REST calls go through `api.js` under `API_BASE`; the sockets
 * and the eyepiece pages are named here so a route is spelled once.
 */
import type {CameraRole} from '../composables/api.types.js'

export const API_BASE = '/api'

export const WS = {
    STREAM: '/ws/stream',
    EYEPIECE: '/ws/eyepiece',
    EYEPIECE_QUALITY: '/ws/eyepiece_quality',
    EVENTS: '/ws/events',
} as const

export const PAGES = {
    EYEPIECE: '/eyepiece',
    EYEPIECE_QUALITY: '/eyepiece_quality',
} as const

export function isEyepiecePage(path: string): boolean {
    return path === PAGES.EYEPIECE || path === PAGES.EYEPIECE_QUALITY
}

/** `/`'s image socket for one camera; the source is fixed per socket. */
export function liveStreamEndpoint(source: CameraRole): string {
    return source === 'guide' ? `${WS.STREAM}?source=guide` : WS.STREAM
}

/** The socket an eyepiece page shows: lossless on `/eyepiece_quality`, JPEG otherwise. */
export function eyepieceStreamEndpoint(path: string): string {
    return path === PAGES.EYEPIECE_QUALITY ? WS.EYEPIECE_QUALITY : WS.EYEPIECE
}
