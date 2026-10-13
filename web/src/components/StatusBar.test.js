import {describe, it, expect, vi} from 'vitest'
import {ref} from 'vue'
import StatusBar from './StatusBar.vue'
import {mountWithDefaults} from '../test/mount.js'
import {createCaptureEvents} from '../composables/events/captureEvents.js'

describe('StatusBar', () => {
    function mountStatusBar(provide = {}) {
        return mountWithDefaults(StatusBar, {provide})
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

        it('shows a capture paused for a camera reconnect as still capturing', () => {
            const wrapper = mountStatusBar({
                eventStream: {captureState: ref('Recovering')},
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
                    rejectedCount: ref(2),
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

        it('names the most recent rejection reason in the tooltip', () => {
            const wrapper = mountStatusBar({
                eventStream: {
                    frameCount: ref(42),
                    rejectedCount: ref(9),
                    lastRejectionReason: ref('stars far larger than the session median'),
                },
            })

            const info = wrapper.findComponent({name: 'BaseInfoIcon'})
            expect(info.exists()).toBe(true)
            expect(info.props('message')).toContain(
                'Most recent rejection: stars far larger than the session median.'
            )
        })

        it('leaves the tooltip alone until something has been rejected', () => {
            const wrapper = mountStatusBar({
                eventStream: {frameCount: ref(42)},
            })

            const info = wrapper.findComponent({name: 'BaseInfoIcon'})
            expect(info.props('message')).not.toContain('Most recent rejection')
        })

        it('shows only total when stacking is disabled (Live View)', () => {
            const wrapper = mountStatusBar({
                eventStream: {
                    frameCount: ref(42),
                    stackedCount: ref(0),
                    rejectedCount: ref(0),
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

            expect(wrapper.find('.tone-error').exists()).toBe(false)
        })

        it('shows error message when lastError is set', () => {
            const wrapper = mountStatusBar({
                eventStream: {lastError: ref('Camera disconnected')},
            })

            const error = wrapper.find('.tone-error')
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

            await wrapper.find('.tone-error').trigger('click')

            expect(clearError).toHaveBeenCalled()
        })
    })

    describe('Unresponsive Warning Display', () => {
        it('does not show warning when unresponsiveWarning is null', () => {
            const wrapper = mountStatusBar({
                eventStream: {unresponsiveWarning: ref(null)},
            })

            expect(wrapper.find('.tone-error').exists()).toBe(false)
        })

        it('shows one row, not two, when an error accompanies the warning', () => {
            const wrapper = mountStatusBar({
                eventStream: {
                    unresponsiveWarning: ref('Ares-C PRO has stopped responding.'),
                    lastError: ref("Camera 'Ares-C PRO' stopped responding — disconnecting"),
                },
            })

            // Both describe the same incident; stacking them just doubles the noise.
            expect(wrapper.findAll('.tone-error')).toHaveLength(1)
            expect(wrapper.find('.tone-error').text()).toContain('has stopped responding')
        })

        it('shows the resume notice after a capture picks back up', () => {
            const wrapper = mountStatusBar({
                eventStream: {
                    resumeNotice: ref('Ares-C PRO is back. Capture resumed with 514 frames still stacked.'),
                },
            })

            const resumed = wrapper.find('.resumed')
            expect(resumed.exists()).toBe(true)
            expect(resumed.text()).toContain('514 frames still stacked')
        })

        it('shows why Focus/Finder mode switched off, and dismisses it on click', async () => {
            const clearFocusModeNotice = vi.fn()
            const wrapper = mountStatusBar({
                eventStream: {
                    focusModeNotice: ref(
                        'Focus/Finder mode turned off: stacking needs the sensor corrections it holds off.',
                    ),
                    clearFocusModeNotice,
                },
            })

            const notice = wrapper.find('.focus-mode-left')
            expect(notice.text()).toContain('Focus/Finder mode turned off')
            await notice.trigger('click')
            expect(clearFocusModeNotice).toHaveBeenCalled()
        })

        it('shows warning message when unresponsiveWarning is set', () => {
            const wrapper = mountStatusBar({
                eventStream: {unresponsiveWarning: ref('Camera is persistently unresponsive')},
            })

            const error = wrapper.find('.tone-error')
            expect(error.exists()).toBe(true)
            expect(error.text()).toContain('Camera is persistently unresponsive')
        })

        it('calls clearUnresponsiveWarning when clicking on warning', async () => {
            const clearUnresponsiveWarning = vi.fn()
            const wrapper = mountStatusBar({
                eventStream: {
                    unresponsiveWarning: ref('Some warning'),
                    clearUnresponsiveWarning,
                },
            })

            await wrapper.find('.tone-error').trigger('click')

            expect(clearUnresponsiveWarning).toHaveBeenCalled()
        })
    })

    describe('Push-To status', () => {
        it('does not dress a blocker up as the last solve result', () => {
            // `solvingMessage` now returns the live blocker ahead of the last verdict,
            // but the element that renders it is still keyed on `lastResult` — so a
            // scope being pushed away from M31 was announced with the green tick and
            // the `success` class of the solve it has just invalidated.
            const wrapper = mountStatusBar({
                eventStream: {
                    plateSolving: ref({
                        inProgress: false,
                        targetName: 'M31',
                        lastResult: 'success',
                    }),
                    solvingMessage: ref('Telescope is moving'),
                    pushToBlocked: ref('Telescope is moving'),
                },
            })

            const result = wrapper.find('.solve-result')
            expect(result.exists() && result.classes().includes('success')).toBe(false)
            expect(wrapper.text()).toContain('Telescope is moving')
        })
    })

    describe('Push-To chip', () => {
        const solving = (overrides) => ({inProgress: false, targetName: 'M31', lastResult: null, ...overrides})

        it('shows a running solve ahead of a blocker and the last result, with a spinner', () => {
            const wrapper = mountStatusBar({
                eventStream: {
                    plateSolving: ref(solving({inProgress: true, lastResult: 'failed'})),
                    solvingMessage: ref('Searching: M31'),
                    pushToBlocked: ref('Telescope is moving'),
                },
            })

            expect(wrapper.findAll('.status-chip')).toHaveLength(1)
            expect(wrapper.find('.solving').text()).toBe('Searching: M31')
            expect(wrapper.find('.solving .base-spinner').exists()).toBe(true)
        })

        it.each([
            ['success', 'tone-success'],
            ['cancelled', 'tone-muted'],
            ['failed', 'tone-error'],
        ])('tints a %s result and clears it on a click', async (result, tone) => {
            const clearPlateSolving = vi.fn()
            const wrapper = mountStatusBar({
                eventStream: {plateSolving: ref(solving({lastResult: result})), solvingMessage: ref('M31'), clearPlateSolving},
            })

            const chip = wrapper.find('.solve-result')
            expect(chip.classes()).toContain(tone)
            await chip.trigger('click')
            expect(clearPlateSolving).toHaveBeenCalled()
        })

        it('cannot dismiss a blocker: it clears when the situation does', async () => {
            const wrapper = mountStatusBar({eventStream: {pushToBlocked: ref('No target selected')}})

            expect(wrapper.find('.push-to-blocked .btn-dismiss').exists()).toBe(false)
        })
    })

    describe('Notices', () => {
        it('dismisses the slow-disk warning on a click', async () => {
            const clearDiskWriterWarning = vi.fn()
            const wrapper = mountStatusBar({eventStream: {diskWriterWarning: ref(12), clearDiskWriterWarning}})

            const notice = wrapper.find('.disk-warning')
            expect(notice.text()).toContain('Slow disk: 12 frames queued')
            expect(notice.classes()).toContain('tone-warning')
            await notice.trigger('click')
            expect(clearDiskWriterWarning).toHaveBeenCalled()
        })

        it('shows every notice at once, in a fixed order', () => {
            const wrapper = mountStatusBar({
                eventStream: {
                    diskWriterWarning: ref(3),
                    resumeNotice: ref('Capture resumed'),
                    focusModeNotice: ref('Focus mode off'),
                    lastError: ref('Boom'),
                },
            })

            const texts = wrapper.findAll('.status-chip-text').map((chip) => chip.text())
            expect(texts).toEqual(['Slow disk: 3 frames queued', 'Capture resumed', 'Focus mode off', 'Boom'])
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

        it('shows the share of delivered frames, not just the count', () => {
            const wrapper = mountStatusBar({
                eventStream: {droppedCount: ref(35), deliveredCount: ref(100)},
            })

            expect(wrapper.find('.dropped').text()).toContain('35%')
        })

        it('shows the bare count while the server has reported no denominator', () => {
            const wrapper = mountStatusBar({
                eventStream: {droppedCount: ref(5), deliveredCount: ref(0)},
            })

            const text = wrapper.find('.dropped').text()
            expect(text).toContain('Dropped: 5')
            expect(text).not.toContain('%')
        })
    })

    // A page opened between captures gets the counts from the sync snapshot alone.
    it('shows the last session\'s counts on a page opened while idle', () => {
        const capture = createCaptureEvents()
        capture.handlers.state_changed({state: 'Idle'})
        capture.handlers.capture_counts({
            frame_count: 120, stacked_count: 100, rejected_count: 20,
            dropped_count: 6, delivered_count: 126, last_rejection_reason: 'trailed stars',
        })

        const wrapper = mountStatusBar({eventStream: {...capture.state}})

        expect(wrapper.find('.frames').text()).toContain('Rejected 20 | Total 120')
        expect(wrapper.find('.dropped').text()).toContain('Dropped: 6')
        expect(wrapper.findComponent({name: 'BaseInfoIcon'}).props('message')).toContain('trailed stars')
    })
})
