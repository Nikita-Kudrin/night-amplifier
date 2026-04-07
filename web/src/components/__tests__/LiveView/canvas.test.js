import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {nextTick} from 'vue'
import {setupMocks, mountLiveView, createMockFrameData} from './setup.js'

describe('LiveView - Canvas Display', () => {
    let mocks

    beforeEach(() => {
        mocks = setupMocks()
    })

    afterEach(() => {
        vi.restoreAllMocks()
    })

    it('shows canvas when frameData is available', async () => {
        mocks.mockImageStream.frameData.value = createMockFrameData(2, 2)
        mocks.mockImageStream.dimensions.value = {width: 2, height: 2}
        const wrapper = mountLiveView()

        await nextTick()

        expect(wrapper.find('.live-canvas').exists()).toBe(true)
        expect(wrapper.find('.live-canvas').isVisible()).toBe(true)
    })

    it('hides placeholder when frame is shown', async () => {
        mocks.mockImageStream.frameData.value = createMockFrameData(2, 2)
        mocks.mockImageStream.dimensions.value = {width: 2, height: 2}
        const wrapper = mountLiveView()

        await nextTick()

        expect(wrapper.find('.placeholder').exists()).toBe(false)
    })

    it('initializes WebGL context on mount', async () => {
        const wrapper = mountLiveView()
        await nextTick()

        expect(mocks.mockWebGLRenderer.init).toHaveBeenCalled()
    })
})
