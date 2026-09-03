import {describe, it, expect, beforeEach, afterEach} from 'vitest'
import {nextTick} from 'vue'
import {
    MockWebSocket,
    createTestContext,
    cleanupTestContext,
    getWebSocket,
    openWebSocket,
    sendEvent,
    suppressConsoleErrors,
    setupGlobalWebSocketMock,
} from './webSocketTestUtils.js'

setupGlobalWebSocketMock()

import {useEventStream as originalUseEventStream} from '../useWebSocket.js'
import { mount } from '@vue/test-utils'

let currentApp = null;
function useEventStream() {
    let result;
    currentApp = mount({
        setup() {
            result = originalUseEventStream()
            return () => {}
        }
    })
    return result
}

describe('useEventStream', () => {
    beforeEach(createTestContext)
    afterEach(() => {
        cleanupTestContext()
        if (currentApp) {
            currentApp.unmount()
            currentApp = null
        }
    })

    it('connects to /ws/events', () => {
        useEventStream()

        expect(MockWebSocket.instances).toHaveLength(1)
        expect(getWebSocket().url).toContain('/ws/events')
    })

    describe('capture events', () => {
        it('updates captureState on state_changed event', async () => {
            const {captureState} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'state_changed', state: 'Capturing'})

            expect(captureState.value).toBe('Capturing')
        })

        it('updates frameCount and stackedCount on frame_captured event', async () => {
            const {frameCount, stackedCount} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'frame_captured', frame_number: 42, stacked_count: 40})

            expect(frameCount.value).toBe(42)
            expect(stackedCount.value).toBe(40)
        })

        it('updates frameCount and stackedCount on frame_rejected event', async () => {
            const {frameCount, stackedCount} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'frame_rejected', frame_number: 15, stacked_count: 10, reason: 'Bad alignment'})

            expect(frameCount.value).toBe(15)
            expect(stackedCount.value).toBe(10)
        })
    })

    describe('error handling', () => {
        it('updates lastError on error event', async () => {
            const {lastError} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'error', message: 'Camera disconnected'})

            expect(lastError.value).toBe('Camera disconnected')
        })

        it('clearError clears lastError', async () => {
            const {lastError, clearError} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'error', message: 'Some error'})
            expect(lastError.value).toBe('Some error')

            clearError()
            expect(lastError.value).toBe(null)
        })

        // The payload shape here must match what the server actually sends —
        // see the assertions in src/server/tests/events.rs. This test used to
        // send `camera_name`, a field the server has never emitted, so it
        // passed green while the UI rendered "Camera undefined".
        it('updates unresponsiveWarning on camera_persistently_unresponsive event', async () => {
            const {unresponsiveWarning} = useEventStream()

            await openWebSocket()
            await sendEvent({
                type: 'camera_persistently_unresponsive',
                name: 'TestCam',
                consecutive_timeouts: 3,
            })

            expect(unresponsiveWarning.value).toBe('TestCam has stopped responding.')
        })

        it('clearUnresponsiveWarning clears unresponsiveWarning', async () => {
            const {unresponsiveWarning, clearUnresponsiveWarning} = useEventStream()

            await openWebSocket()
            await sendEvent({
                type: 'camera_persistently_unresponsive',
                name: 'TestCam',
                consecutive_timeouts: 3,
            })
            expect(unresponsiveWarning.value).toBeTruthy()

            clearUnresponsiveWarning()
            expect(unresponsiveWarning.value).toBe(null)
        })

        it('reports reconnect progress and how it ended', async () => {
            const {unresponsiveWarning} = useEventStream()

            await openWebSocket()
            await sendEvent({
                type: 'camera_reconnecting',
                name: 'TestCam',
                attempt: 2,
                of: 5,
                next_attempt_in_s: 10,
            })
            expect(unresponsiveWarning.value).toBe('Reconnecting to TestCam — attempt 2 of 5.')

            await sendEvent({
                type: 'camera_reconnect_failed',
                name: 'TestCam',
                attempts: 5,
                reason: 'still unreachable after 300 s',
            })
            expect(unresponsiveWarning.value).toContain('Could not bring TestCam back')
        })

        it('clears the fault warnings when the capture resumes', async () => {
            const {unresponsiveWarning, resumeNotice} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'camera_persistently_unresponsive', name: 'TestCam'})
            await sendEvent({type: 'capture_resumed', name: 'TestCam', stacked_count: 514})

            expect(unresponsiveWarning.value).toBe(null)
            expect(resumeNotice.value).toContain('514 frames still stacked')
        })

        it('handles malformed JSON gracefully', async () => {
            const {lastEvent} = useEventStream()
            const consoleSpy = suppressConsoleErrors()

            await openWebSocket()
            getWebSocket().simulateMessage('not valid json')
            await nextTick()

            expect(consoleSpy).toHaveBeenCalled()
            expect(lastEvent.value).toBe(null)

            consoleSpy.mockRestore()
        })
    })

    describe('general events', () => {
        it('stores lastEvent for any event type', async () => {
            const {lastEvent} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'settings_updated'})

            expect(lastEvent.value).toEqual({type: 'settings_updated'})
        })

        it('handles camera_connected event', async () => {
            const {lastEvent} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'camera_connected', name: 'Neptune-C II'})

            expect(lastEvent.value).toEqual({type: 'camera_connected', name: 'Neptune-C II'})
        })
    })

    describe('ASTAP installation events', () => {
        const astapEventTestCases = [
            {
                name: 'astap_install_starting',
                event: {type: 'astap_install_starting', component: 'D80 Database'},
                expected: {component: 'D80 Database', stage: 'starting'},
            },
            {
                name: 'astap_install_progress',
                event: {
                    type: 'astap_install_progress',
                    component: 'D80 Database',
                    bytes_downloaded: 52428800,
                    total_bytes: 1261887242,
                    percent: 4.15,
                    stage: 'Downloading Database',
                    overall_percent: 52.08,
                },
                expected: {
                    component: 'D80 Database',
                    stage: 'downloading',
                    percent: 4.15,
                    bytesDownloaded: 52428800,
                    totalBytes: 1261887242,
                    stageName: 'Downloading Database',
                    overallPercent: 52.08,
                },
            },
            {
                name: 'astap_install_extracting',
                event: {
                    type: 'astap_install_extracting',
                    component: 'D80 Database',
                    progress: 45.5,
                    stage: 'Extracting Database',
                    overall_percent: 72.75,
                },
                expected: {
                    component: 'D80 Database',
                    stage: 'extracting',
                    percent: 45.5,
                    stageName: 'Extracting Database',
                    overallPercent: 72.75,
                },
            },
            {
                name: 'astap_install_completed',
                event: {
                    type: 'astap_install_completed',
                    component: 'D80 Database',
                    stage: 'Database Installed',
                    overall_percent: 100,
                },
                expected: {
                    component: 'D80 Database',
                    stage: 'completed',
                    stageName: 'Database Installed',
                    overallPercent: 100,
                },
            },
            {
                name: 'astap_install_failed',
                event: {
                    type: 'astap_install_failed',
                    component: 'D80 Database',
                    error: 'Download timeout',
                },
                expected: {component: 'D80 Database', stage: 'failed', error: 'Download timeout'},
            },
        ]

        it.each(astapEventTestCases)(
            'updates astapInstallProgress on $name event',
            async ({event, expected}) => {
                const {astapInstallProgress} = useEventStream()

                await openWebSocket()
                await sendEvent(event)

                expect(astapInstallProgress.value).not.toBe(null)
                for (const [key, value] of Object.entries(expected)) {
                    expect(astapInstallProgress.value[key]).toBe(value)
                }
            }
        )

        it('clearAstapInstallProgress clears astapInstallProgress', async () => {
            const {astapInstallProgress, clearAstapInstallProgress} = useEventStream()

            await openWebSocket()
            await sendEvent({
                type: 'astap_install_progress',
                component: 'D80 Database',
                bytes_downloaded: 100,
                total_bytes: 1000,
                percent: 10,
            })

            expect(astapInstallProgress.value).not.toBe(null)

            clearAstapInstallProgress()
            expect(astapInstallProgress.value).toBe(null)
        })
    })

    describe('Push-To events', () => {
        it('updates plateSolving on plate_solving_started event', async () => {
            const {plateSolving} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'plate_solving_started', target_name: 'M31'})

            expect(plateSolving.value.inProgress).toBe(true)
            expect(plateSolving.value.targetName).toBe('M31')
        })

        it('names the current strategy while a multi-attempt solve runs', async () => {
            // A cold solve works down several strategies, the last of which can run for
            // a minute. The status line has to move, or a slow solve reads as a hang.
            const {plateSolving, solvingMessage} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'plate_solving_started', target_name: 'M31'})
            await sendEvent({
                type: 'plate_solving_progress',
                stage: 'last known pointing',
                attempt: 2,
                total: 4,
            })

            expect(plateSolving.value.inProgress).toBe(true)
            expect(plateSolving.value.targetName).toBe('M31')
            expect(solvingMessage.value).toBe('Searching (2/4: last known pointing) : M31')
        })

        it('does not clutter the status line when there is only one attempt', async () => {
            const {solvingMessage} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'plate_solving_started', target_name: 'M31'})
            await sendEvent({
                type: 'plate_solving_progress',
                stage: 'full sky (blind FOV)',
                attempt: 1,
                total: 1,
            })

            expect(solvingMessage.value).toBe('Searching : M31')
        })

        it('drops the strategy label once the solve finishes', async () => {
            const {plateSolving, solvingMessage} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'plate_solving_started', target_name: 'M31'})
            await sendEvent({
                type: 'plate_solving_progress', stage: 'selected target', attempt: 2, total: 3,
            })
            await sendEvent({type: 'position_solved'})

            expect(plateSolving.value.stage).toBe(null)
            expect(solvingMessage.value).toBe('Found : M31')
        })

        it('clears plateSolving on target_cleared event', async () => {
            const {plateSolving} = useEventStream()

            await openWebSocket()
            // Set initial state
            await sendEvent({type: 'plate_solving_started', target_name: 'M31'})
            expect(plateSolving.value.inProgress).toBe(true)

            // Send target_cleared
            await sendEvent({type: 'target_cleared'})
            expect(plateSolving.value.inProgress).toBe(false)
            expect(plateSolving.value.targetName).toBe(null)
        })

        it('reports a cancel as a cancel, not as a failure', async () => {
            // The server used to answer every cancel with `position_solve_failed`, so
            // an ordinary settings save rendered as "Failed to find M31" and made a
            // still-good last position look untrustworthy.
            const {plateSolving, solvingMessage} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'plate_solving_started', target_name: 'M31'})
            expect(plateSolving.value.inProgress).toBe(true)

            await sendEvent({type: 'plate_solving_cancelled'})

            expect(plateSolving.value.inProgress).toBe(false)
            expect(plateSolving.value.lastResult).toBe('cancelled')
            expect(solvingMessage.value).toBe('Solving cancelled')
        })

        it('surfaces why Push-To is idle', async () => {
            // Every one of these branches used to be a `debug!` and a silent return,
            // which is exactly what "I installed ASTAP and nothing happens" looked
            // like from the outside.
            const {pushToBlocked, solvingMessage} = useEventStream()

            await openWebSocket()
            await sendEvent({
                type: 'push_to_blocked',
                reason: 'ASTAP or its star database is not installed',
            })

            expect(pushToBlocked.value).toBe('ASTAP or its star database is not installed')
            expect(solvingMessage.value).toBe('ASTAP or its star database is not installed')
        })

        it('clears the blocked notice once solving resumes', async () => {
            const {pushToBlocked} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'push_to_blocked', reason: 'No target selected'})
            expect(pushToBlocked.value).toBe('No target selected')

            await sendEvent({type: 'plate_solving_started', target_name: 'M31'})
            expect(pushToBlocked.value).toBe(null)
        })

        it('clears the blocked notice when the server says nothing is blocking', async () => {
            const {pushToBlocked} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'push_to_blocked', reason: 'No target selected'})
            await sendEvent({type: 'push_to_blocked', reason: null})

            expect(pushToBlocked.value).toBe(null)
        })

        it('says the scope is moving instead of leaving the last fix on screen', async () => {
            // The report: push the scope and the status bar went on reading
            // "Found : M31" the whole way, because a solve ending was the only thing
            // that ever wrote to it. The movement states are now reported, and being
            // newer than the last verdict they have to win.
            const {solvingMessage} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'plate_solving_started', target_name: 'M31'})
            await sendEvent({type: 'position_solved'})
            expect(solvingMessage.value).toBe('Found : M31')

            await sendEvent({type: 'push_to_blocked', reason: 'Telescope is moving'})
            expect(solvingMessage.value).toBe('Telescope is moving')

            await sendEvent({type: 'push_to_blocked', reason: 'Waiting for the view to settle'})
            expect(solvingMessage.value).toBe('Waiting for the view to settle')
        })

        it('shows the retry countdown instead of hiding it behind the failure', async () => {
            // `position_solve_failed` arrives after `push_to_blocked`, and the failure
            // branch used to come first in the message, so the backoff notice the
            // server had just sent was never displayed at all.
            const {solvingMessage} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'plate_solving_started', target_name: 'M31'})
            await sendEvent({type: 'position_solve_failed', reason: 'no match'})
            expect(solvingMessage.value).toBe('Failed to find : M31')

            await sendEvent({
                type: 'push_to_blocked',
                reason: 'Waiting before the next solve attempt',
            })
            expect(solvingMessage.value).toBe('Waiting before the next solve attempt')
        })

        it('falls back to the last verdict once nothing is blocking any more', async () => {
            // The blocker outranks the verdict only while it is live. When the server
            // says nothing is blocking, the last thing that actually happened is again
            // the most useful thing to show.
            const {solvingMessage} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'plate_solving_started', target_name: 'M31'})
            await sendEvent({type: 'position_solved'})
            await sendEvent({type: 'push_to_blocked', reason: 'Telescope is moving'})
            await sendEvent({type: 'push_to_blocked', reason: null})

            expect(solvingMessage.value).toBe('Found : M31')
        })

        it('keeps showing the search while a solve is running', async () => {
            // A blocker left over from before the solve started must not displace the
            // one thing the user most wants to see.
            const {solvingMessage} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'push_to_blocked', reason: 'Telescope is moving'})
            await sendEvent({type: 'plate_solving_started', target_name: 'M31'})

            expect(solvingMessage.value).toBe('Searching : M31')
        })

        it('shows solving again after an equipment change restarts it', async () => {
            // Issue 3: an equipment change aborts the solve in flight. The UI must not
            // be left showing a stale result for a solve that no longer applies.
            const {plateSolving} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'plate_solving_started', target_name: 'M31'})
            await sendEvent({type: 'position_solve_failed', reason: 'no match'})
            expect(plateSolving.value.lastResult).toBe('failed')

            await sendEvent({
                type: 'plate_solving_restarted',
                reason: 'Equipment settings changed',
            })
            expect(plateSolving.value.lastResult).toBe(null)

            await sendEvent({type: 'plate_solving_started', target_name: 'M31'})
            expect(plateSolving.value.inProgress).toBe(true)
        })
    })
})
