import {computed, toValue} from 'vue'

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
        const fl = optics?.focal_length_mm
        const py = optics?.pixel_size_y_um
        const h = optics?.sensor_height_px
        if (!fl || !py || !h) return toValue(pushDirection)?.fovDeg || 0

        const effectiveFl = fl * (optics.barlow_coeff || 1)
        const sensorHeightMm = (h * py) / 1000
        return (2 * Math.atan(sensorHeightMm / (2 * effectiveFl)) * 180) / Math.PI
    })
}
