import {describe, it, expect, beforeEach} from 'vitest'
import {flushPromises} from '@vue/test-utils'
import {lastImageStreamOptions, mountLiveView, setupMocks} from './setup.js'

describe('LiveView guide camera source', () => {
    beforeEach(() => {
        setupMocks()
    })

    it('hides the source toggle when no guide camera is connected', () => {
        const wrapper = mountLiveView({hasGuideCamera: false})
        expect(wrapper.find('.guide-toggle').exists()).toBe(false)
    })

    it('offers the toggle once a guide camera is connected, off by default', () => {
        const wrapper = mountLiveView({hasGuideCamera: true})

        const toggle = wrapper.find('.guide-toggle')
        expect(toggle.exists()).toBe(true)
        expect(toggle.text()).toContain('Guide camera')
        expect(toggle.classes()).not.toContain('active')
        expect(toggle.attributes('aria-pressed')).toBe('false')
    })

    /// The endpoint is a getter, so the socket can move to the other source rather than
    /// the view needing a second stream.
    it('points the stream at the guide source when the toggle is on', async () => {
        const wrapper = mountLiveView({hasGuideCamera: true})

        const endpoint = lastImageStreamOptions().endpoint
        expect(typeof endpoint === 'function' || 'value' in Object(endpoint)).toBe(true)

        const read = () => (typeof endpoint === 'function' ? endpoint() : endpoint.value)
        expect(read()).toBe('/ws/stream')

        await wrapper.find('.guide-toggle').trigger('click')
        await flushPromises()

        expect(read()).toBe('/ws/stream?source=guide')
        expect(wrapper.find('.guide-toggle').classes()).toContain('active')
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

    /// The zoom cluster is a separate group; adding the source switch beside it must not
    /// change what that group contains.
    it('leaves the zoom controls untouched', () => {
        const wrapper = mountLiveView({hasGuideCamera: true})
        const zoomControls = wrapper.find('.zoom-controls')
        expect(zoomControls.findAll('.btn-overlay').length).toBe(2)
    })
})
