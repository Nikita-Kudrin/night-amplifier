import {computed, toValue} from 'vue'
import {fieldOfViewDeg} from '../utils/optics.js'

/**
 * Field of view of the camera on screen, in degrees of image height.
 *
 * The solve reports the field it was planned against, which with a guide camera connected
 * is the guide scope's. The Push-To chevron places an off-target arrow against the frame
 * edge, so it must describe the picture it is drawn over: the displayed camera's own
 * optics profile, then the telescope settings, then the solve's field as a last resort.
 *
 * @param {import('vue').MaybeRefOrGetter<{name?: string}|null>} camera - The displayed camera
 * @param {import('vue').MaybeRefOrGetter<object|null>} settings - Capture settings
 * @param {import('vue').MaybeRefOrGetter<{fovDeg?: number}|null>} pushDirection - Latest push direction
 */
export function useDisplayedFov(camera, settings, pushDirection) {
    return computed(() => {
        const current = toValue(settings)
        const name = toValue(camera)?.name
        const optics = (name && current?.camera_telescope_profiles?.[name]) || current?.telescope
        const fov = fieldOfViewDeg(
            optics?.sensor_height_px,
            optics?.pixel_size_y_um,
            optics?.focal_length_mm,
            optics?.barlow_coeff
        )
        return fov ?? (toValue(pushDirection)?.fovDeg || 0)
    })
}

/**
 * The camera a Push-To chevron was oriented for, when that is not the camera on screen.
 *
 * `angle_deg` carries the solving camera's rotation and parity, and with a guide camera
 * connected that is the guide's even while the imaging camera is shown. Over another
 * camera's picture the chevron could point anywhere, so the view drops it and says
 * whose it is. Empty when it matches, or when either camera is unknown.
 *
 * @param {import('vue').MaybeRefOrGetter<{name?: string}|null>} camera - The displayed camera
 * @param {import('vue').MaybeRefOrGetter<{solvedCamera?: string|null}|null>} pushDirection - Latest push direction
 */
export function useOtherSolvedCamera(camera, pushDirection) {
    return computed(() => {
        const solved = toValue(pushDirection)?.solvedCamera
        const shown = toValue(camera)?.name
        return solved && shown && solved !== shown ? solved : ''
    })
}
