import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {nextTick} from 'vue'
import {setupMocks, mountLiveView, createMockFrameData} from './setup.js'

describe('LiveView - Touch Interactions', () => {
    let mocks

    beforeEach(() => {
        mocks = setupMocks()
    })

    afterEach(() => {
        vi.restoreAllMocks()
    })

    it('handles single touch for panning', async () => {
        mocks.mockImageStream.frameData.value = createMockFrameData(2, 2)
        mocks.mockImageStream.dimensions.value = {width: 2, height: 2}
        const wrapper = mountLiveView()

        await nextTick()

        const liveView = wrapper.find('.live-view')

        // Start touch
        await liveView.trigger('touchstart', {
            touches: [{clientX: 100, clientY: 100}],
        })

        // Move touch
        await liveView.trigger('touchmove', {
            touches: [{clientX: 150, clientY: 150}],
        })

        // The canvas should have a transform applied
        const canvas = wrapper.find('.live-canvas')
        expect(canvas.element.style.transform).toContain('translate')
    })

    it('handles two-finger touch for pinch zoom', async () => {
        mocks.mockImageStream.frameData.value = createMockFrameData(2, 2)
        mocks.mockImageStream.dimensions.value = {width: 2, height: 2}
        const wrapper = mountLiveView()

        await nextTick()

        const liveView = wrapper.find('.live-view')

        // Start pinch
        await liveView.trigger('touchstart', {
            touches: [
                {clientX: 100, clientY: 100},
                {clientX: 200, clientY: 200},
            ],
        })

        // Spread fingers apart
        await liveView.trigger('touchmove', {
            touches: [
                {clientX: 50, clientY: 50},
                {clientX: 250, clientY: 250},
            ],
        })

        // Scale should have changed
        const canvas = wrapper.find('.live-canvas')
        expect(canvas.element.style.transform).toContain('scale')
    })
})
