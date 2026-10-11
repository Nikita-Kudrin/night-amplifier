import {describe, it, expect} from 'vitest'
import {mount} from '@vue/test-utils'
import {nextTick, ref} from 'vue'
import SelectedCameraTag from './SelectedCameraTag.vue'

const MAIN = {id: 'cam-a', name: 'ASI533MC Pro', role: 'main'}
const GUIDE = {id: 'cam-b', name: 'ASI120MM Mini', role: 'guide'}

function mountTag(selected, cameras = [MAIN, GUIDE], props = {}) {
    const selectedCamera = ref(selected)
    const wrapper = mount(SelectedCameraTag, {
        props,
        global: {provide: {cameras: ref(cameras), selectedCamera}},
    })
    return {wrapper, selectedCamera}
}

describe('SelectedCameraTag', () => {
    it('names the selected camera and its role', () => {
        expect(mountTag('cam-b').wrapper.text()).toBe('Guide · ASI120MM Mini')
        expect(mountTag('cam-a').wrapper.text()).toBe('Main · ASI533MC Pro')
    })

    it('follows the selection made on the Main tab', async () => {
        const {wrapper, selectedCamera} = mountTag('cam-a')
        selectedCamera.value = 'cam-b'
        await nextTick()
        expect(wrapper.text()).toBe('Guide · ASI120MM Mini')
    })

    it('shows the name alone for a camera in no role', () => {
        expect(mountTag('cam-c', [{id: 'cam-c', name: 'Simulator', role: null}]).wrapper.text()).toBe('Simulator')
    })

    it('renders nothing without a selected camera', () => {
        expect(mountTag(null).wrapper.find('[data-test="selected-camera-tag"]').exists()).toBe(false)
    })

    // Equipment edits the last camera's profile with none selected.
    it('names the fallback camera as not connected without a selection', () => {
        expect(mountTag(null, [MAIN], {fallbackName: 'ASI120MM Mini'}).wrapper.text())
            .toBe('ASI120MM Mini · not connected')
    })

    it('prefers the selected camera over the fallback', () => {
        expect(mountTag('cam-a', [MAIN], {fallbackName: 'ASI120MM Mini'}).wrapper.text())
            .toBe('Main · ASI533MC Pro')
    })
})
