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

    // 2026-11-03: three minutes of "Waiting for frames" while the camera was being reopened.
    it('says why no frame comes while recovery works on the camera on screen', () => {
        mocks.mockImageStream.connected.value = true
        const wrapper = mountLiveView({
            captureState: 'Recovering',
            eventStream: {cameraTrouble: {role: 'main', text: 'Ares-C PRO is not sending frames — reopening it…'}},
        })

        const text = wrapper.find('.placeholder').text()
        expect(text).toContain('Ares-C PRO is not sending frames')
        expect(text).not.toContain('Waiting for frames')
    })

    it('keeps waiting for frames when the trouble is the other camera’s', () => {
        mocks.mockImageStream.connected.value = true
        const wrapper = mountLiveView({
            captureState: 'Capturing',
            eventStream: {cameraTrouble: {role: 'guide', text: 'Reconnecting to Neptune-C II…'}},
        })

        expect(wrapper.find('.placeholder').text()).toContain('Waiting for frames')
    })
})
