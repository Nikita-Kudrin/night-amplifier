import {CAPTURE_HELP} from './capture.js'
import {CAMERA_HELP} from './camera.js'
import {EYEPIECE_HELP} from './eyepiece.js'
import {IMAGE_QUALITY_HELP} from './imageQuality.js'
import {PUSH_TO_HELP} from './pushTo.js'
import {STACKING_HELP} from './stacking.js'

/** Every panel's help, by key; one file per panel. */
export const HELP_SECTIONS = Object.freeze({
    capture: CAPTURE_HELP,
    camera: CAMERA_HELP,
    eyepiece: EYEPIECE_HELP,
    imageQuality: IMAGE_QUALITY_HELP,
    pushTo: PUSH_TO_HELP,
    stacking: STACKING_HELP,
})

/** Help tooltips and copy for the UI, by key. `help.test.js` keeps the keys unique. */
export const HELP_TEXTS = Object.freeze(Object.assign({}, ...Object.values(HELP_SECTIONS)))
