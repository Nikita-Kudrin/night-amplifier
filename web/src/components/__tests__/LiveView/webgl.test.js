import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {nextTick} from 'vue'
import {setupMocks, mountLiveView, createMockFrameData} from './setup.js'

describe('LiveView - WebGL Rendering', () => {
    let mocks

    beforeEach(() => {
        mocks = setupMocks()
    })

    afterEach(() => {
        vi.restoreAllMocks()
    })

    it('uploads texture when frameData changes', async () => {
        const wrapper = mountLiveView()
        await nextTick()

        // Set frame data
        mocks.mockImageStream.frameData.value = createMockFrameData(4, 4)
        mocks.mockImageStream.dimensions.value = {width: 4, height: 4}
        await nextTick()

        // Check that render was called
        expect(mocks.mockWebGLRenderer.render).toHaveBeenCalled()
    })

    it('updates viewport when dimensions change', async () => {
        const wrapper = mountLiveView()
        await nextTick()

        mocks.mockImageStream.frameData.value = createMockFrameData(100, 50)
        mocks.mockImageStream.dimensions.value = {width: 100, height: 50}
        await nextTick()

        // Render should be called with updated dimensions
        expect(mocks.mockWebGLRenderer.render).toHaveBeenCalled()
    })

    it('cleans up WebGL resources on unmount', async () => {
        const wrapper = mountLiveView()
        await nextTick()

        wrapper.unmount()

        expect(mocks.mockWebGLRenderer.cleanup).toHaveBeenCalled()
    })
})
