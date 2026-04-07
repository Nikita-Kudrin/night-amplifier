import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {nextTick} from 'vue'
import {setupMocks, mountLiveView, createMockFrameData} from './setup.js'

describe('LiveView - Zoom Controls', () => {
    let mocks

    beforeEach(() => {
        mocks = setupMocks()
    })

    afterEach(() => {
        vi.restoreAllMocks()
    })

    it('renders zoom control buttons', async () => {
        mocks.mockImageStream.frameData.value = createMockFrameData(2, 2)
        mocks.mockImageStream.dimensions.value = {width: 2, height: 2}
        const wrapper = mountLiveView()

        await nextTick()

        const zoomControls = wrapper.find('.zoom-controls')
        expect(zoomControls.exists()).toBe(true)

        const buttons = zoomControls.findAll('.btn-overlay')
        expect(buttons.length).toBe(5) // zoom in, zoom out, fit, reset, fullscreen
    })

    it('displays current zoom level', async () => {
        mocks.mockImageStream.frameData.value = createMockFrameData(2, 2)
        mocks.mockImageStream.dimensions.value = {width: 2, height: 2}
        const wrapper = mountLiveView()

        await nextTick()

        expect(wrapper.find('.zoom-level').text()).toBe('100%')
    })

    it('displays frame number', async () => {
        mocks.mockImageStream.frameData.value = createMockFrameData(2, 2)
        mocks.mockImageStream.dimensions.value = {width: 2, height: 2}
        mocks.mockImageStream.frameNumber.value = 42
        const wrapper = mountLiveView()

        await nextTick()

        expect(wrapper.find('.frame-number').text()).toContain('42')
    })

    it('displays render backend info', async () => {
        mocks.mockImageStream.frameData.value = createMockFrameData(2, 2)
        mocks.mockImageStream.dimensions.value = {width: 2, height: 2}
        const wrapper = mountLiveView()

        await nextTick()

        const backendInfo = wrapper.find('.render-backend')
        expect(backendInfo.exists()).toBe(true)
        // Should show webgl2-10bit since mock supports it
        expect(backendInfo.text()).toContain('webgl2-10bit')
    })
})

describe('LiveView - View Reset', () => {
    let mocks

    beforeEach(() => {
        mocks = setupMocks()
    })

    afterEach(() => {
        vi.restoreAllMocks()
    })

    it('resets scale and position when reset button clicked', async () => {
        mocks.mockImageStream.frameData.value = createMockFrameData(2, 2)
        mocks.mockImageStream.dimensions.value = {width: 2, height: 2}
        const wrapper = mountLiveView()

        await nextTick()

        // First zoom in
        const liveView = wrapper.find('.live-view')
        await liveView.trigger('wheel', {deltaY: -100, preventDefault: vi.fn()})

        // Then click reset (4th button)
        const buttons = wrapper.findAll('.btn-overlay')
        await buttons[3].trigger('click')

        // Should be back to 100%
        expect(wrapper.find('.zoom-level').text()).toBe('100%')
    })
})

describe('LiveView - Fullscreen', () => {
    let mocks

    beforeEach(() => {
        mocks = setupMocks()
    })

    afterEach(() => {
        vi.restoreAllMocks()
    })

    it('calls requestFullscreen when fullscreen button clicked', async () => {
        mocks.mockImageStream.frameData.value = createMockFrameData(2, 2)
        mocks.mockImageStream.dimensions.value = {width: 2, height: 2}
        const wrapper = mountLiveView()

        await nextTick()

        // Fullscreen is the last button
        const buttons = wrapper.findAll('.btn-overlay')
        await buttons[buttons.length - 1].trigger('click')

        expect(Element.prototype.requestFullscreen).toHaveBeenCalled()
    })
})
