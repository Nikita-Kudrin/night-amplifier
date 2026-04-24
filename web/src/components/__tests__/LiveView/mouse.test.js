import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {nextTick} from 'vue'
import {setupMocks, mountLiveView, createMockFrameData} from './setup.js'
import LiveViewControls from '../../LiveViewControls.vue'

describe('LiveView - Mouse Interactions', () => {
    let mocks

    beforeEach(() => {
        mocks = setupMocks()
    })

    afterEach(() => {
        vi.restoreAllMocks()
    })

    it('updates cursor to grabbing on mousedown', async () => {
        mocks.mockImageStream.frameData.value = createMockFrameData(2, 2)
        mocks.mockImageStream.dimensions.value = {width: 2, height: 2}
        const wrapper = mountLiveView()

        await nextTick()

        const liveView = wrapper.find('.live-view')
        await liveView.trigger('mousedown', {button: 0})

        const canvas = wrapper.find('.live-canvas')
        expect(canvas.element.style.cursor).toBe('grabbing')
    })

    it('handles wheel event for zoom', async () => {
        mocks.mockImageStream.frameData.value = createMockFrameData(2, 2)
        mocks.mockImageStream.dimensions.value = {width: 2, height: 2}
        const wrapper = mountLiveView()

        await nextTick()

        const liveView = wrapper.find('.live-view')

        // Zoom in (negative deltaY)
        await liveView.trigger('wheel', {deltaY: -100, preventDefault: vi.fn()})

        // Check scale changed in LiveViewControls
        const controls = wrapper.findComponent(LiveViewControls)
        expect(controls.props('scale')).not.toBe(1.0)
    })
})

