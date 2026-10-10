import {describe, it, expect, beforeEach, vi} from 'vitest'
import {ref} from 'vue'
import {flushPromises} from '@vue/test-utils'
import {lastImageStreamOptions, mountLiveView, setupMocks} from './setup.js'

vi.mock('../../../composables/api.js', () => ({
    setViewedCamera: vi.fn(async (camera) => ({camera})),
}))

import {setViewedCamera} from '../../../composables/api.js'
import {getAppState} from '../../../composables/useAppState.js'

function readEndpoint() {
    const endpoint = lastImageStreamOptions().endpoint
    return typeof endpoint === 'function' ? endpoint() : endpoint.value
}

describe('LiveView guide camera source', () => {
    beforeEach(() => {
        setupMocks()
        setViewedCamera.mockClear()
        setViewedCamera.mockImplementation(async (camera) => ({camera}))
        getAppState().clearGlobalError()
    })

    it('hides the source toggle when no guide camera is connected', () => {
        const wrapper = mountLiveView({hasGuideCamera: false})
        expect(wrapper.find('.guide-toggle').exists()).toBe(false)
    })

    it('offers the toggle once a guide camera is connected, off while the server views the imaging camera', () => {
        const wrapper = mountLiveView({hasGuideCamera: true})

        const toggle = wrapper.find('.guide-toggle')
        expect(toggle.exists()).toBe(true)
        expect(toggle.text()).toContain('Guide camera')
        expect(toggle.classes()).not.toContain('active')
        expect(toggle.attributes('aria-pressed')).toBe('false')
    })

    /// A reloaded page shows the toggle where the operator left it.
    it('starts on when the server already views the guide camera', () => {
        const wrapper = mountLiveView({hasGuideCamera: true, viewedCamera: 'guide'})

        expect(wrapper.find('.guide-toggle').classes()).toContain('active')
        expect(readEndpoint()).toBe('/ws/stream?source=guide')
    })

    /// The endpoint is a getter, so the socket can move to the other source rather than
    /// the view needing a second stream.
    it('asks the server to view the guide camera and follows its answer', async () => {
        const wrapper = mountLiveView({hasGuideCamera: true})
        expect(readEndpoint()).toBe('/ws/stream')

        await wrapper.find('.guide-toggle').trigger('click')
        await flushPromises()

        expect(setViewedCamera).toHaveBeenCalledWith('guide')
        expect(readEndpoint()).toBe('/ws/stream?source=guide')
        expect(wrapper.find('.guide-toggle').classes()).toContain('active')

        await wrapper.find('.guide-toggle').trigger('click')
        await flushPromises()

        expect(setViewedCamera).toHaveBeenLastCalledWith('main')
        expect(readEndpoint()).toBe('/ws/stream')
    })

    /// Two operators: the other one's later toggle arrives as an event while this request
    /// is out, and this answer — read before that toggle — must not undo it here.
    it('keeps an event that overtook its own answer', async () => {
        const viewedCamera = ref('main')
        const viewedCameraRevision = ref(0)
        setViewedCamera.mockImplementationOnce(async (camera) => {
            viewedCamera.value = camera
            viewedCameraRevision.value++
            viewedCamera.value = 'main' // the other operator's toggle
            viewedCameraRevision.value++
            return {camera}
        })
        const wrapper = mountLiveView({hasGuideCamera: true, eventStream: {viewedCamera, viewedCameraRevision}})

        await wrapper.find('.guide-toggle').trigger('click')
        await flushPromises()

        expect(viewedCamera.value).toBe('main')
        expect(readEndpoint()).toBe('/ws/stream')
    })

    it('stays off and says why when the server refuses', async () => {
        setViewedCamera.mockRejectedValueOnce(new Error('No guide camera is connected'))
        const wrapper = mountLiveView({hasGuideCamera: true})

        await wrapper.find('.guide-toggle').trigger('click')
        await flushPromises()

        expect(wrapper.find('.guide-toggle').classes()).not.toContain('active')
        expect(readEndpoint()).toBe('/ws/stream')
        expect(getAppState().globalError.value).toContain('No guide camera')
    })

    /// The server falls back when the guide camera goes away; the toggle follows.
    it('turns off when the server falls back to the imaging camera', async () => {
        const viewedCamera = ref('guide')
        const wrapper = mountLiveView({hasGuideCamera: true, eventStream: {viewedCamera}})
        expect(readEndpoint()).toBe('/ws/stream?source=guide')

        viewedCamera.value = 'main'
        await flushPromises()

        expect(wrapper.find('.guide-toggle').classes()).not.toContain('active')
        expect(readEndpoint()).toBe('/ws/stream')
    })

    /// A stale `guide` with no guide camera must not point the view at a dead stream.
    it('shows the imaging camera while no guide camera is connected, whatever the event said', () => {
        mountLiveView({hasGuideCamera: false, viewedCamera: 'guide'})
        expect(readEndpoint()).toBe('/ws/stream')
    })

    /// Push-To chevrons are drawn over whichever stream is on screen, so the arrow must
    /// survive the switch rather than being torn down with the old source.
    it('keeps the guide arrow mounted across a source switch', async () => {
        const wrapper = mountLiveView({
            hasGuideCamera: true,
            currentTarget: {name: 'M42'},
            pushDirection: {angleDeg: 45, distanceDeg: 2, isClose: false, directionHint: 'up', fovDeg: 1.2},
        })

        expect(wrapper.findComponent({name: 'GuideArrow'}).exists()).toBe(true)

        await wrapper.find('.guide-toggle').trigger('click')
        await flushPromises()

        expect(wrapper.findComponent({name: 'GuideArrow'}).exists()).toBe(true)
    })

    it('places the arrow against the field of the camera on screen', async () => {
        const guideOptics = {focal_length_mm: 120, pixel_size_y_um: 2.9, sensor_height_px: 1080}
        const wrapper = mountLiveView({
            hasGuideCamera: true,
            viewedCamera: 'guide',
            guideCamera: {name: 'Guide Cam'},
            settings: {camera_telescope_profiles: {'Guide Cam': guideOptics}},
            currentTarget: {name: 'M42'},
            pushDirection: {angleDeg: 45, distanceDeg: 2, isClose: false, directionHint: 'up', fovDeg: 0.5},
        })

        const expected = (2 * Math.atan((1080 * 2.9) / 1000 / 240) * 180) / Math.PI
        expect(wrapper.findComponent({name: 'GuideArrow'}).props('fovDeg')).toBeCloseTo(expected, 6)
    })

    /// The zoom cluster is a separate group; adding the source switch beside it must not
    /// change what that group contains.
    it('leaves the zoom controls untouched', () => {
        const wrapper = mountLiveView({hasGuideCamera: true})
        const zoomControls = wrapper.find('.zoom-controls')
        expect(zoomControls.findAll('.btn-overlay').length).toBe(2)
    })
})
