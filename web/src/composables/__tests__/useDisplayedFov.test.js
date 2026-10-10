import {describe, it, expect} from 'vitest'
import {ref} from 'vue'
import {useDisplayedFov} from '../useDisplayedFov.js'

/** 2 * atan(h * py / 2000 / fl) in degrees, the field the composable must report. */
function fieldOf({focal_length_mm: fl, pixel_size_y_um: py, sensor_height_px: h, barlow_coeff: b = 1}) {
    return (2 * Math.atan((h * py) / 1000 / (2 * fl * b)) * 180) / Math.PI
}

const IMAGING = {focal_length_mm: 400, pixel_size_y_um: 3.76, sensor_height_px: 3008}
const GUIDE = {focal_length_mm: 120, pixel_size_y_um: 2.9, sensor_height_px: 1080}

describe('useDisplayedFov', () => {
    it('uses the displayed camera\'s own optics profile', () => {
        const settings = ref({telescope: IMAGING, camera_telescope_profiles: {'Guide Cam': GUIDE}})
        const camera = ref({name: 'Guide Cam'})

        const fov = useDisplayedFov(camera, settings, ref({fovDeg: 0.5}))

        expect(fov.value).toBeCloseTo(fieldOf(GUIDE), 6)
        camera.value = {name: 'Imaging Cam'}
        expect(fov.value).toBeCloseTo(fieldOf(IMAGING), 6)
    })

    it('applies the Barlow to the focal length', () => {
        const optics = {...IMAGING, barlow_coeff: 2}
        const fov = useDisplayedFov(ref(null), ref({telescope: optics}), ref(null))
        expect(fov.value).toBeCloseTo(fieldOf(optics), 6)
    })

    it('falls back to the solve\'s field when no optics are known', () => {
        const fov = useDisplayedFov(ref({name: 'X'}), ref({telescope: {focal_length_mm: 400}}), ref({fovDeg: 1.25}))
        expect(fov.value).toBe(1.25)
    })

    it('is zero with nothing to go on', () => {
        expect(useDisplayedFov(ref(null), ref(null), ref(null)).value).toBe(0)
    })
})
