import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {mount} from '@vue/test-utils'
import {setupMocks, mountLiveView} from './setup'
import LiveView from '../../LiveView.vue'
import {contextProvides} from '../../../composables/appContext'
import type {Mocks} from './setup'

describe('LiveView - Placeholder State', () => {
    let mocks: Mocks

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

    // The stream's accessor answers null without a provide; the view still renders.
    it('renders without an event stream', () => {
        const wrapper = mount(LiveView, {global: {provide: contextProvides({})}})

        expect(wrapper.find('.placeholder').text()).toContain('Start capture')
        expect(wrapper.findComponent({name: 'GuideArrow'}).exists()).toBe(false)
    })

    it('shows "Waiting for frames..." when capturing but no frame yet', () => {
        mocks.mockImageStream.connected.value = true
        const wrapper = mountLiveView({captureState: 'Capturing'})

        expect(wrapper.find('.placeholder').text()).toContain('Waiting for frames')
    })
})
