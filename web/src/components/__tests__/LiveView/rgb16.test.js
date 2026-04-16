import {nextTick} from 'vue'
import {setupMocks, mountLiveView} from './setup.js'

describe('LiveView - RGB8 Data Handling', () => {
    let mocks

    beforeEach(() => {
        mocks = setupMocks()
    })

    afterEach(() => {
        vi.restoreAllMocks()
    })

    it('uploads RGB8 data natively when WebGL2 is supported', async () => {
        mountLiveView()
        await nextTick()

        const pixelData = new Uint8Array(3) // 1 pixel * 3 bytes
        mocks.mockImageStream.frameData.value = pixelData
        mocks.mockImageStream.dimensions.value = {width: 1, height: 1}
        await nextTick()

        expect(mocks.mockWebGLRenderer.render).toHaveBeenCalled()
        const renderCall =
            mocks.mockWebGLRenderer.render.mock.calls[
            mocks.mockWebGLRenderer.render.mock.calls.length - 1
                ]
        expect(renderCall[1]).toBe(pixelData)
    })

    it('falls back to 8-bit when backend logic defaults to webgl2-8bit', async () => {
        mocks.mockWebGLRenderer.backend.value = 'webgl2-8bit'

        const wrapper = mountLiveView()
        await nextTick()

        const pixelData = new Uint8Array(3) // 1 pixel * 3 bytes
        mocks.mockImageStream.frameData.value = pixelData
        mocks.mockImageStream.dimensions.value = {width: 1, height: 1}
        await nextTick()

        expect(mocks.mockWebGLRenderer.render).toHaveBeenCalled()
        expect(wrapper.find('.render-backend').text()).toContain('8-bit')
    })
})
