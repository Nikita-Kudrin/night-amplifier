import {describe, it, expect, vi} from 'vitest'
import {mount} from '@vue/test-utils'
import {ref} from 'vue'
import StatusBar from './StatusBar.vue'

describe('StatusBar', () => {
    // Helper to create mock provides
    function createMockProvides(overrides = {}) {
        return {
            eventStream: {
                connected: ref(true),
                captureState: ref('Idle'),
                frameCount: ref(0),
                stackedCount: ref(0),
                droppedCount: ref(0),
                lastError: ref(null),
                clearError: vi.fn(),
                diskWriterWarning: ref(null),
                clearDiskWriterWarning: vi.fn(),
                plateSolving: ref({inProgress: false, targetName: null, lastResult: null}),
                clearPlateSolving: vi.fn(),
                ...overrides.eventStream,
            },
            selectedCamera: ref(overrides.selectedCamera ?? null),
            cameras: ref(overrides.cameras ?? []),
        }
    }

    function mountStatusBar(provides = {}) {
        return mount(StatusBar, {
            global: {
                provide: createMockProvides(provides),
            },
        })
    }

    describe('Connection Status', () => {
        it('shows connected status when eventStream is connected', () => {
            const wrapper = mountStatusBar()

            const connection = wrapper.find('.connection')
            expect(connection.classes()).toContain('connected')
            expect(connection.text()).toContain('Connected')
        })

        it('shows disconnected status when eventStream is not connected', () => {
            const wrapper = mountStatusBar({
                eventStream: {connected: ref(false)},
            })

            const connection = wrapper.find('.connection')
            expect(connection.classes()).toContain('disconnected')
            expect(connection.text()).toContain('Disconnected')
        })
    })

    describe('Camera Info', () => {
        it('shows camera name when camera is selected', () => {
            const wrapper = mountStatusBar({
                selectedCamera: 'cam1',
                cameras: [{id: 'cam1', name: 'Neptune-C II', connected: true}],
            })

            expect(wrapper.text()).toContain('Neptune-C II')
        })

        it('does not show camera info when no camera selected', () => {
            const wrapper = mountStatusBar({
                selectedCamera: null,
            })

            expect(wrapper.find('.camera').exists()).toBe(false)
        })
    })

    describe('Capture State', () => {
        it('shows Idle state with correct styling', () => {
            const wrapper = mountStatusBar({
                eventStream: {captureState: ref('Idle')},
            })

            const state = wrapper.find('.state')
            expect(state.classes()).toContain('idle')
            expect(state.text()).toContain('Idle')
        })

        it('shows Capturing state with correct styling', () => {
            const wrapper = mountStatusBar({
                eventStream: {captureState: ref('Capturing')},
            })

            const state = wrapper.find('.state')
            expect(state.classes()).toContain('capturing')
            expect(state.text()).toContain('Capturing')
        })

        it('shows Starting state with correct styling', () => {
            const wrapper = mountStatusBar({
                eventStream: {captureState: ref('Starting')},
            })

            const state = wrapper.find('.state')
            expect(state.classes()).toContain('starting')
            expect(state.text()).toContain('Starting')
        })

        it('shows Stopping state with correct styling', () => {
            const wrapper = mountStatusBar({
                eventStream: {captureState: ref('Stopping')},
            })

            const state = wrapper.find('.state')
            expect(state.classes()).toContain('stopping')
            expect(state.text()).toContain('Stopping')
        })

        it('shows Error state with correct styling', () => {
            const wrapper = mountStatusBar({
                eventStream: {captureState: ref('Error')},
            })

            const state = wrapper.find('.state')
            expect(state.classes()).toContain('error')
            expect(state.text()).toContain('Error')
        })
    })

    describe('Frame Counter', () => {
        it('does not show frame counter when no frames captured', () => {
            const wrapper = mountStatusBar({
                eventStream: {frameCount: ref(0)},
            })

            expect(wrapper.find('.frames').exists()).toBe(false)
        })

        it('shows frame counter when frames have been captured', () => {
            const wrapper = mountStatusBar({
                eventStream: {
                    frameCount: ref(42),
                    stackedCount: ref(40),
                },
            })

            const frames = wrapper.find('.frames')
            expect(frames.exists()).toBe(true)
            expect(frames.text()).toContain('Rejected 2')
            expect(frames.text()).toContain('Total 42')
        })

        it('shows only total when no frames rejected', () => {
            const wrapper = mountStatusBar({
                eventStream: {
                    frameCount: ref(42),
                    stackedCount: ref(42),
                },
            })

            const frames = wrapper.find('.frames')
            expect(frames.exists()).toBe(true)
            expect(frames.text()).toContain('Total 42')
            expect(frames.text()).not.toContain('Rejected')
        })
    })

    describe('Error Display', () => {
        it('does not show error when lastError is null', () => {
            const wrapper = mountStatusBar({
                eventStream: {lastError: ref(null)},
            })

            expect(wrapper.find('.error').exists()).toBe(false)
        })

        it('shows error message when lastError is set', () => {
            const wrapper = mountStatusBar({
                eventStream: {lastError: ref('Camera disconnected')},
            })

            const error = wrapper.find('.error')
            expect(error.exists()).toBe(true)
            expect(error.text()).toContain('Camera disconnected')
        })

        it('calls clearError when clicking on error', async () => {
            const clearError = vi.fn()
            const wrapper = mountStatusBar({
                eventStream: {
                    lastError: ref('Some error'),
                    clearError,
                },
            })

            await wrapper.find('.error').trigger('click')

            expect(clearError).toHaveBeenCalled()
        })
    })

    describe('Dropped Frames Counter', () => {
        it('does not show dropped counter when no frames dropped', () => {
            const wrapper = mountStatusBar({
                eventStream: {droppedCount: ref(0)},
            })

            expect(wrapper.find('.dropped').exists()).toBe(false)
        })

        it('shows dropped counter when frames have been dropped', () => {
            const wrapper = mountStatusBar({
                eventStream: {droppedCount: ref(5)},
            })

            const dropped = wrapper.find('.dropped')
            expect(dropped.exists()).toBe(true)
            expect(dropped.text()).toContain('Dropped: 5')
        })
    })
})
