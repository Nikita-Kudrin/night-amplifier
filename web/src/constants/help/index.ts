import {CAPTURE_HELP} from './capture'
import {CAMERA_HELP} from './camera'
import {EYEPIECE_HELP} from './eyepiece'
import {IMAGE_QUALITY_HELP} from './imageQuality'
import {PUSH_TO_HELP} from './pushTo'
import {STACKING_HELP} from './stacking'

/** Every panel's help, by key; one file per panel. */
export const HELP_SECTIONS = {
    capture: CAPTURE_HELP,
    camera: CAMERA_HELP,
    eyepiece: EYEPIECE_HELP,
    imageQuality: IMAGE_QUALITY_HELP,
    pushTo: PUSH_TO_HELP,
    stacking: STACKING_HELP,
} as const

/** Help tooltips and copy for the UI, by key. `constants/index.test.ts` keeps the keys unique. */
export const HELP_TEXTS = {
    ...CAPTURE_HELP,
    ...CAMERA_HELP,
    ...EYEPIECE_HELP,
    ...IMAGE_QUALITY_HELP,
    ...PUSH_TO_HELP,
    ...STACKING_HELP,
} as const
