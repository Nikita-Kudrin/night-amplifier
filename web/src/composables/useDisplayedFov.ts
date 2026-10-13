import {computed, toValue} from 'vue'
import type {DeepReadonly, MaybeRefOrGetter} from 'vue'
import {fieldOfViewDeg} from '../utils/optics.js'
import type {Camera, Settings} from './api.types.js'
import type {ClientPushDirection} from './events/pushToEvents.js'

type Named = Pick<Camera, 'name'> | null | undefined
type Optics = DeepReadonly<Pick<Settings, 'telescope' | 'camera_telescope_profiles'>> | null | undefined

/**
 * Field of view of the camera on screen, in degrees of image height.
 *
 * The solve reports the field it was planned against, which with a guide camera connected
 * is the guide scope's. The Push-To chevron places an off-target arrow against the frame
 * edge, so it must describe the picture it is drawn over: the displayed camera's own
 * optics profile, then the telescope settings, then the solve's field as a last resort.
 */
export function useDisplayedFov(
    camera: MaybeRefOrGetter<Named>,
    settings: MaybeRefOrGetter<Optics>,
    pushDirection: MaybeRefOrGetter<Pick<ClientPushDirection, 'fovDeg'> | null>,
) {
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
        return fov ?? (toValue(pushDirection)?.fovDeg ?? 0)
    })
}

/**
 * The camera a Push-To chevron was oriented for, when that is not the camera on screen.
 *
 * `angle_deg` carries the solving camera's rotation and parity, and with a guide camera
 * connected that is the guide's even while the imaging camera is shown. Over another
 * camera's picture the chevron could point anywhere, so the view drops it and says
 * whose it is. Empty when it matches, or when either camera is unknown.
 */
export function useOtherSolvedCamera(
    camera: MaybeRefOrGetter<Named>,
    pushDirection: MaybeRefOrGetter<Pick<ClientPushDirection, 'solvedCamera'> | null>,
) {
    return computed(() => {
        const solved = toValue(pushDirection)?.solvedCamera
        const shown = toValue(camera)?.name
        return solved && shown && solved !== shown ? solved : ''
    })
}
