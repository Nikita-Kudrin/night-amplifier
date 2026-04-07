import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {setupMocks, mountLiveView} from './setup.js'

describe('LiveView - Placeholder State', () => {
    let mocks

    beforeEach(() => {
        mocks = setupMocks()
    })

    afterEach(() => {
        vi.restoreAllMocks()
    })

    it('shows placeholder when no frame data', () => {
        const wrapper = mountLiveView()

        expect(wrapper.find('.placeholder').exists()).toBe(true)
        // Canvas uses v-show so it exists but is hidden
        const canvas = wrapper.find('.live-canvas')
        expect(canvas.exists()).toBe(true)
        expect(canvas.attributes('style')).toContain('display: none')
    })

    it('shows "Connecting to stream..." when not connected', () => {
        mocks.mockImageStream.connected.value = false
        const wrapper = mountLiveView()

        expect(wrapper.find('.placeholder').text()).toContain('Connecting to stream')
    })

    it('shows "Start capture..." when idle and connected', () => {
        mocks.mockImageStream.connected.value = true
        const wrapper = mountLiveView({captureState: 'Idle'})

        expect(wrapper.find('.placeholder').text()).toContain('Start capture')
    })

    it('shows "Waiting for frames..." when capturing but no frame yet', () => {
        mocks.mockImageStream.connected.value = true
        const wrapper = mountLiveView({captureState: 'Capturing'})

        expect(wrapper.find('.placeholder').text()).toContain('Waiting for frames')
    })
})
