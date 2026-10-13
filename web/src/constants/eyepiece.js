/** The eyepiece pages: their tone controls, stream size and zoom. */
import {PREVIEW_RESOLUTION_OPTIONS} from './imageQuality.js'
import {serverDefault, serverLimits} from './serverSettings.js'

/** Black level (`eyepiece.intensity`), which the renderer clamps where it reads it. */
export const BLACK_LEVEL_LIMITS = Object.freeze({
    min: 0.0,
    max: 1.0,
    step: 0.05,
    default: serverDefault('eyepiece.intensity'),
})

/**
 * The darkening half is anchored to the sky, so its reach is only meaningful against
 * where the sky actually renders: the server stops it where the spatial darkening
 * saturates (`BLACK_FLOOR_RANGE`). The lifting half stops at 15%, short of the server's.
 */
export const BLACK_FLOOR_LIMITS = serverLimits('eyepiece.black_floor', {max: 0.15, step: 0.005})

/**
 * The one RGB8+LZ4 size every client of `/eyepiece_quality` receives. No 1080p: the
 * server's `EyepieceStreamResolution` refuses it.
 * @type {{value: import('../composables/api.types.js').EyepieceStreamResolution, label: string}[]}
 */
export const EYEPIECE_STREAM_RESOLUTION_OPTIONS = PREVIEW_RESOLUTION_OPTIONS.filter(
    (opt) => opt.value !== 'hd1080'
)

// The eyepiece zooms up to 2x and never out past the screen.
export const EYEPIECE_ZOOM = {
    max: 2,
    wheelSensitivity: 0.001,
}
