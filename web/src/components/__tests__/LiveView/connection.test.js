import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {nextTick} from 'vue'
import {setupMocks, mountLiveView} from './setup.js'

describe('LiveView - Connection Status', () => {
    let mocks

    beforeEach(() => {
        mocks = setupMocks()
    })

    afterEach(() => {
        vi.restoreAllMocks()
    })

    it('shows disconnected status when not connected', async () => {
        mocks.mockImageStream.connected.value = false
        const wrapper = mountLiveView()

        await nextTick()

        const status = wrapper.find('.connection-status')
        expect(status.exists()).toBe(true)
        expect(status.classes()).toContain('disconnected')
        expect(status.text()).toContain('Disconnected')
    })

    it('hides disconnected status when connected', async () => {
        mocks.mockImageStream.connected.value = true
        const wrapper = mountLiveView()

        await nextTick()

        expect(wrapper.find('.connection-status').exists()).toBe(false)
    })
})
