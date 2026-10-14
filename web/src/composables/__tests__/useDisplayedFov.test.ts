import {describe, it, expect} from 'vitest'
import {ref} from 'vue'
import {useDisplayedFov, useOtherSolvedCamera} from '../useDisplayedFov'
import type {TelescopeSettings} from '../api.types'

/** Optics with only `fields` known. */
function optics(fields: Partial<TelescopeSettings>): TelescopeSettings {
    return {
        focal_length_mm: null, pixel_size_x_um: null, pixel_size_y_um: null,
        sensor_width_px: null, sensor_height_px: null, barlow_coeff: null, ...fields,
    }
}

type Known = TelescopeSettings & {[K in 'focal_length_mm' | 'pixel_size_y_um' | 'sensor_height_px']: number}

/** 2 * atan(h * py / 2000 / fl) in degrees, the field the composable must report. */
function fieldOf({focal_length_mm: fl, pixel_size_y_um: py, sensor_height_px: h, barlow_coeff: b}: Known) {
    return (2 * Math.atan((h * py) / 1000 / (2 * fl * (b ?? 1))) * 180) / Math.PI
}

const IMAGING = {...optics({}), focal_length_mm: 400, pixel_size_y_um: 3.76, sensor_height_px: 3008}
const GUIDE = {...optics({}), focal_length_mm: 120, pixel_size_y_um: 2.9, sensor_height_px: 1080}

describe('useDisplayedFov', () => {
    it('uses the displayed camera\'s own optics profile', () => {
        const settings = ref({telescope: IMAGING, camera_telescope_profiles: {'Guide Cam': GUIDE}})
        const camera = ref({name: 'Guide Cam'})

        const fov = useDisplayedFov(camera, settings, ref({fovDeg: 0.5, solvedCamera: 'Guide Cam'}))

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
        const fov = useDisplayedFov(ref({name: 'X'}), ref({telescope: optics({focal_length_mm: 400})}), ref({fovDeg: 1.25, solvedCamera: 'X'}))
        expect(fov.value).toBe(1.25)
        const unattributed = useDisplayedFov(ref({name: 'X'}), ref(null), ref({fovDeg: 1.25, solvedCamera: null}))
        expect(unattributed.value).toBe(1.25)
    })

    it('does not borrow another camera\'s solved field', () => {
        // 2026-10-12: the Main view, no focal length set, counted screens in the guide's 1.45°
        const settings = ref({telescope: optics({}), camera_telescope_profiles: {'Ares-C PRO': optics({pixel_size_y_um: 3.76})}})
        const camera = ref({name: 'Ares-C PRO'})
        const fov = useDisplayedFov(camera, settings, ref({fovDeg: 1.45, solvedCamera: 'Neptune-C II'}))

        expect(fov.value).toBe(0)
        camera.value = {name: 'Neptune-C II'}
        expect(fov.value).toBe(1.45)
    })

    it('is zero with nothing to go on', () => {
        expect(useDisplayedFov(ref(null), ref(null), ref(null)).value).toBe(0)
    })
})

describe('useOtherSolvedCamera', () => {
    const BOTH = [{name: 'Ares-C PRO'}, {name: 'Neptune-C II'}]

    it('names the solving camera while another camera is on screen', () => {
        const camera = ref({name: 'Ares-C PRO'})
        const direction = ref({solvedCamera: 'Neptune-C II'})

        const other = useOtherSolvedCamera(camera, direction, BOTH)

        expect(other.value).toEqual({name: 'Neptune-C II', connected: true})
        camera.value = {name: 'Neptune-C II'}
        expect(other.value).toBeNull()
    })

    it('says when the solving camera has disconnected', () => {
        // 2026-10-14: the guide left, and the imaging view still said "Arrow on" it
        const connected = ref<{name: string}[]>(BOTH)
        const other = useOtherSolvedCamera(ref({name: 'Ares-C PRO'}), ref({solvedCamera: 'Neptune-C II'}), connected)

        connected.value = [{name: 'Ares-C PRO'}]

        expect(other.value).toEqual({name: 'Neptune-C II', connected: false})
    })

    it('follows a new solve from the camera on screen', () => {
        const direction = ref({solvedCamera: 'Neptune-C II'})
        const other = useOtherSolvedCamera(ref({name: 'Ares-C PRO'}), direction, BOTH)

        direction.value = {solvedCamera: 'Ares-C PRO'}

        expect(other.value).toBeNull()
    })

    it('keeps the chevron when either camera is unknown', () => {
        expect(useOtherSolvedCamera(ref({name: 'Ares-C PRO'}), ref({solvedCamera: null}), BOTH).value).toBeNull()
        expect(useOtherSolvedCamera(ref(null), ref({solvedCamera: 'Neptune-C II'}), BOTH).value).toBeNull()
        expect(useOtherSolvedCamera(ref({name: 'Ares-C PRO'}), ref(null), BOTH).value).toBeNull()
    })
})
