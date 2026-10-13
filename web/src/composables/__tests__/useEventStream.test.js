import {describe, it, expect, beforeEach, afterEach, vi} from 'vitest'
import {effectScope, nextTick} from 'vue'
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

// Opening the socket loads the Push-To status; unmocked, every test fetched it for real.
vi.mock('../api.js', async (importOriginal) =>
    (await import('../../test/mockApi.js')).mockApi(await importOriginal()))

import {ApiError, getPushToStatus} from '../api.js'
import {pushToStatus} from '../../test/fixtures.js'
import {useEventStream as originalUseEventStream} from '../useEventStream.js'
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

        it('keeps the session counters when a paused capture resumes', async () => {
            const {captureState, frameCount, stackedCount} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'state_changed', state: 'Capturing'})
            await sendEvent({type: 'frame_captured', frame_number: 42, stacked_count: 40})
            await sendEvent({type: 'state_changed', state: 'Recovering'})
            await sendEvent({type: 'state_changed', state: 'Starting'})

            expect(captureState.value).toBe('Starting')
            expect(frameCount.value).toBe(42)
            expect(stackedCount.value).toBe(40)
        })

        it('zeroes the counters for a fresh start', async () => {
            const {frameCount, stackedCount} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'frame_captured', frame_number: 42, stacked_count: 40})
            await sendEvent({type: 'state_changed', state: 'Idle'})
            await sendEvent({type: 'state_changed', state: 'Starting'})

            expect(frameCount.value).toBe(0)
            expect(stackedCount.value).toBe(0)
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

        // A guide camera, or an idle one, has no capture to resume — nothing else would
        // ever take the warning down after it came back.
        it('clears the reconnect warning once that camera is back', async () => {
            const {unresponsiveWarning} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'camera_reconnecting', name: 'Guide', attempt: 3, of: 31, next_attempt_in_s: 5})
            await sendEvent({type: 'camera_phase_changed', name: 'Guide', phase: 'recovering'})
            await sendEvent({type: 'camera_phase_changed', name: 'Imaging', phase: 'idle'})
            expect(unresponsiveWarning.value).toContain('Reconnecting to Guide')

            await sendEvent({type: 'camera_phase_changed', name: 'Guide', phase: 'guiding'})
            expect(unresponsiveWarning.value).toBe(null)
        })

        // Two bodies of one model share a name; the imaging camera coming back to idle
        // must not take down the warning about the guide camera still reconnecting.
        it('keeps the reconnect warning while only a twin in the other role is back', async () => {
            const {unresponsiveWarning} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'camera_reconnecting', name: 'ASI120MM', role: 'guide', attempt: 3, of: 31, next_attempt_in_s: 5})
            await sendEvent({type: 'camera_phase_changed', name: 'ASI120MM', role: 'main', phase: 'idle'})
            expect(unresponsiveWarning.value).toContain('Reconnecting to ASI120MM')

            await sendEvent({type: 'camera_phase_changed', name: 'ASI120MM', role: 'guide', phase: 'guiding'})
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

        it('explains why Focus/Finder mode switched off, until dismissed', async () => {
            const {focusModeNotice, clearFocusModeNotice} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'focus_mode_left'})
            expect(focusModeNotice.value).toContain('Focus/Finder mode turned off')

            clearFocusModeNotice()
            expect(focusModeNotice.value).toBe(null)
        })

        it('follows the viewed camera, starting on the imaging camera', async () => {
            const {viewedCamera, viewedCameraRevision} = useEventStream()
            expect(viewedCamera.value).toBe('main')

            await openWebSocket()
            await sendEvent({type: 'viewed_camera_changed', camera: 'guide'})
            expect(viewedCamera.value).toBe('guide')

            await sendEvent({type: 'viewed_camera_changed', camera: 'main'})
            expect(viewedCamera.value).toBe('main')
            expect(viewedCameraRevision.value).toBe(2)
        })

        it('handles malformed JSON gracefully', async () => {
            const stream = useEventStream()
            const handler = vi.fn()
            stream.on('error', handler)
            const consoleSpy = suppressConsoleErrors()

            await openWebSocket()
            getWebSocket().simulateMessage('not valid json')
            await nextTick()

            expect(consoleSpy).toHaveBeenCalled()
            expect(handler).not.toHaveBeenCalled()
            expect(stream.lastError.value).toBeNull()

            consoleSpy.mockRestore()
        })
    })

    describe('disk and drop counters', () => {
        it('shows a backed-up disk writer until it catches up or is dismissed', async () => {
            const {diskWriterWarning, clearDiskWriterWarning} = useEventStream()
            await openWebSocket()

            await sendEvent({type: 'disk_writer_warning', queue_depth: 42})
            expect(diskWriterWarning.value).toBe(42)
            await sendEvent({type: 'disk_writer_warning_cleared'})
            expect(diskWriterWarning.value).toBeNull()

            await sendEvent({type: 'disk_writer_warning', queue_depth: 7})
            clearDiskWriterWarning()
            expect(diskWriterWarning.value).toBeNull()
        })

        // The drop count needs its denominator; an older server sends none.
        it('counts dropped frames against the frames delivered', async () => {
            const {droppedCount, deliveredCount} = useEventStream()
            await openWebSocket()

            await sendEvent({type: 'frame_dropped', dropped_count: 3, delivered_count: 120})
            expect([droppedCount.value, deliveredCount.value]).toEqual([3, 120])
            await sendEvent({type: 'frame_dropped', dropped_count: 4})
            expect([droppedCount.value, deliveredCount.value]).toEqual([4, 0])
        })

        it('says a camera is back until the notice is dismissed', async () => {
            const {resumeNotice, clearResumeNotice} = useEventStream()
            await openWebSocket()

            await sendEvent({type: 'capture_resumed', name: 'ASI533MC Pro', stacked_count: 12})
            expect(resumeNotice.value).toBe('ASI533MC Pro is back. Capture resumed with 12 frames still stacked.')
            clearResumeNotice()
            expect(resumeNotice.value).toBeNull()
        })
    })

    describe('subscriptions', () => {
        it('hands every event of a type to what subscribed to it', async () => {
            const {on} = useEventStream()
            const settings = vi.fn()
            const camera = vi.fn()
            on('settings_updated', settings)
            on('camera_connected', camera)

            await openWebSocket()
            await sendEvent({type: 'camera_connected', name: 'Neptune-C II'})
            await sendEvent({type: 'settings_updated'})

            expect(camera).toHaveBeenCalledExactlyOnceWith({type: 'camera_connected', name: 'Neptune-C II'})
            expect(settings).toHaveBeenCalledExactlyOnceWith({type: 'settings_updated'})
        })

        // The `lastEvent` watcher this replaced ran once per tick, on the last event only.
        it('sees both of two events arriving in one tick', async () => {
            const {on} = useEventStream()
            const phases = []
            on('camera_phase_changed', (event) => phases.push(event.phase))

            await openWebSocket()
            getWebSocket().simulateMessage(JSON.stringify({type: 'camera_phase_changed', name: 'A', phase: 'idle'}))
            getWebSocket().simulateMessage(JSON.stringify({type: 'camera_phase_changed', name: 'A', phase: 'capturing'}))

            expect(phases).toEqual(['idle', 'capturing'])
        })

        it('runs the stream\'s own table before later subscribers, and survives one that throws', async () => {
            const {on, captureState} = useEventStream()
            const seen = []
            const consoleSpy = suppressConsoleErrors()
            on('state_changed', () => {
                throw new Error('broken subscriber')
            })
            on('state_changed', () => seen.push(captureState.value))

            await openWebSocket()
            await sendEvent({type: 'state_changed', state: 'Capturing'})

            expect(seen).toEqual(['Capturing'])
            expect(consoleSpy).toHaveBeenCalled()
            consoleSpy.mockRestore()
        })

        it('stops delivering once unsubscribed, or once the subscribing scope is gone', async () => {
            const {on} = useEventStream()
            const early = vi.fn()
            const scoped = vi.fn()
            const off = on('settings_updated', early)
            const scope = effectScope()
            scope.run(() => on('settings_updated', scoped))

            await openWebSocket()
            off()
            scope.stop()
            await sendEvent({type: 'settings_updated'})

            expect(early).not.toHaveBeenCalled()
            expect(scoped).not.toHaveBeenCalled()
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

    // One casing at the boundary: the installer reads these fields and no snake_case twin.
    describe('catalog installation events', () => {
        it.each([
            {
                event: {type: 'catalog_install_starting'},
                expected: {stage: 'starting', fileName: '', percent: null, bytesDownloaded: 0, totalBytes: null, error: null},
            },
            {
                event: {type: 'catalog_install_progress', file_name: 'NGC.csv', percent: 50, bytes_downloaded: 512, total_bytes: 1024},
                expected: {stage: 'downloading', fileName: 'NGC.csv', percent: 50, bytesDownloaded: 512, totalBytes: 1024, error: null},
            },
            {
                event: {type: 'catalog_install_completed', object_count: 13957},
                expected: {stage: 'completed', objectCount: 13957, error: null},
            },
            {
                event: {type: 'catalog_install_failed', error: 'disk full'},
                expected: {stage: 'failed', error: 'disk full'},
            },
        ])('maps $event.type to camelCase', async ({event, expected}) => {
            const {catalogInstallProgress} = useEventStream()

            await openWebSocket()
            await sendEvent(event)

            expect(catalogInstallProgress.value).toEqual(expected)
        })

        it('keeps the download readout through a file completing', async () => {
            const {catalogInstallProgress} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'catalog_install_progress', file_name: 'NGC.csv', percent: 100, bytes_downloaded: 1024, total_bytes: 1024})
            await sendEvent({type: 'catalog_file_completed', file_name: 'NGC.csv'})

            expect(catalogInstallProgress.value).toMatchObject({stage: 'file_completed', fileName: 'NGC.csv', bytesDownloaded: 1024})
        })
    })

    describe('Push-To status on connect', () => {
        /** Lets `onOpen` finish with the status reply it awaited. */
        async function statusSettled() {
            await getPushToStatus.mock.results.at(-1).value.catch(() => {})
            await nextTick()
        }

        it('shows the target and push direction the server already holds', async () => {
            getPushToStatus.mockResolvedValueOnce(pushToStatus({
                current_target: {designation: 'M31', name: 'Andromeda Galaxy', ra_degrees: 10.68, dec_degrees: 41.27},
                direction: {angle_deg: 120, distance_deg: 4.5, direction_hint: 'SE', is_close: false, solved_camera: 'Main'},
            }))
            const {currentTarget, pushDirection} = useEventStream()

            await openWebSocket()
            await statusSettled()

            expect(currentTarget.value).toMatchObject({designation: 'M31'})
            // The REST status carries no FOV; only the event does.
            expect(pushDirection.value).toEqual({
                angleDeg: 120, distanceDeg: 4.5, directionHint: 'SE', isClose: false,
                fovDeg: 0, solvedCamera: 'Main',
            })
        })

        it('starts empty when the status request fails, and says so', async () => {
            const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
            getPushToStatus.mockRejectedValueOnce(new ApiError('Server unavailable'))
            const {currentTarget, pushDirection} = useEventStream()

            await openWebSocket()
            await statusSettled()

            expect(currentTarget.value).toBeNull()
            expect(pushDirection.value).toBeNull()
            expect(warn).toHaveBeenCalledWith('Push-To status failed:', 'Server unavailable')
            warn.mockRestore()
        })

        it("starts empty without a word on a Community server's refusal", async () => {
            const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
            getPushToStatus.mockRejectedValueOnce(new ApiError('Push-To requires Pro', 403))
            const {currentTarget} = useEventStream()

            await openWebSocket()
            await statusSettled()

            expect(currentTarget.value).toBeNull()
            expect(warn).not.toHaveBeenCalled()
            warn.mockRestore()
        })

        it('shows a solve the server is already running', async () => {
            getPushToStatus.mockResolvedValueOnce(pushToStatus({is_solving: true}))
            const {plateSolving} = useEventStream()

            await openWebSocket()
            await statusSettled()

            expect(plateSolving.value.inProgress).toBe(true)
        })

        // The status left the server before the event did, so it describes an older state.
        it('drops a status answer that a Push-To event overtook', async () => {
            let answer
            getPushToStatus.mockReturnValueOnce(new Promise((resolve) => (answer = resolve)))
            const {currentTarget} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'target_cleared'})
            answer(pushToStatus({current_target: {designation: 'M31', ra_degrees: 10.68, dec_degrees: 41.27}}))
            await statusSettled()

            expect(currentTarget.value).toBeNull()
        })

        // Only the field an event wrote is stale: a page opened mid-solve gets progress
        // events, none of which names the target the status alone carries.
        it('keeps the target and direction of a status answer a solve event overtook', async () => {
            let answer
            getPushToStatus.mockReturnValueOnce(new Promise((resolve) => (answer = resolve)))
            const {currentTarget, pushDirection, plateSolving} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'plate_solving_progress', stage: 'hinted', attempt: 2, total: 4})
            await sendEvent({type: 'push_to_blocked', reason: 'Waiting'})
            answer(pushToStatus({
                current_target: {designation: 'M31', ra_degrees: 10.68, dec_degrees: 41.27},
                direction: {angle_deg: 120, distance_deg: 4.5, direction_hint: 'SE', is_close: false},
                is_solving: false,
            }))
            await statusSettled()

            expect(currentTarget.value).toMatchObject({designation: 'M31'})
            expect(pushDirection.value).toMatchObject({angleDeg: 120})
            expect(plateSolving.value).toMatchObject({inProgress: true, stage: {attempt: 2, total: 4}})
        })

        it('keeps a direction that arrived while the status was in flight', async () => {
            let answer
            getPushToStatus.mockReturnValueOnce(new Promise((resolve) => (answer = resolve)))
            const {currentTarget, pushDirection} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'push_direction_updated', angle_deg: 7, distance_deg: 1, direction_hint: 'N', is_close: false})
            answer(pushToStatus({current_target: {designation: 'M31', ra_degrees: 10.68, dec_degrees: 41.27}}))
            await statusSettled()

            expect(currentTarget.value).toMatchObject({designation: 'M31'})
            expect(pushDirection.value).toMatchObject({angleDeg: 7})
        })

        // `position_solved` was sent while the socket was down; the status read on reopen
        // is all that says the solve is over, and it does not say how it ended.
        it('ends a solve the status says is over, without inventing a verdict', async () => {
            const {plateSolving} = useEventStream()
            await openWebSocket()
            await statusSettled()
            await sendEvent({type: 'plate_solving_started', target_name: 'M31'})
            await sendEvent({type: 'plate_solving_progress', stage: 'blind', attempt: 3, total: 4})

            getPushToStatus.mockResolvedValueOnce(pushToStatus({is_solving: false}))
            getWebSocket().simulateClose()
            vi.advanceTimersByTime(1000)
            MockWebSocket.instances.at(-1).simulateOpen()
            await statusSettled()

            expect(plateSolving.value).toEqual({inProgress: false, targetName: 'M31', lastResult: null, stage: null})
        })

        it('reads the status again whenever the socket reopens', async () => {
            getPushToStatus.mockClear()
            useEventStream()

            await openWebSocket()
            getWebSocket().simulateClose()
            vi.advanceTimersByTime(1000)
            MockWebSocket.instances.at(-1).simulateOpen()
            await nextTick()

            expect(getPushToStatus).toHaveBeenCalledTimes(2)
        })
    })

    describe('Push-To answers', () => {
        it('aims at a target the panel set, dropping the old direction', async () => {
            const {aimAt, currentTarget, pushDirection} = useEventStream()
            await openWebSocket()
            await sendEvent({type: 'push_direction_updated', angle_deg: 1, distance_deg: 2, direction_hint: 'N', is_close: false})

            const target = {designation: 'M42', ra_degrees: 83.8, dec_degrees: -5.4}
            expect(aimAt(target)).toBe(target)

            expect(currentTarget.value).toEqual(target)
            expect(pushDirection.value).toBeNull()
        })

        it('forgets the target, its direction and the solve together', async () => {
            const {forgetTarget, currentTarget, plateSolving, pushToBlocked} = useEventStream()
            await openWebSocket()
            await sendEvent({type: 'target_changed', designation: 'M31', ra_degrees: 10.68, dec_degrees: 41.27})
            await sendEvent({type: 'plate_solving_started', target_name: 'M31'})
            await sendEvent({type: 'push_to_blocked', reason: 'Waiting'})

            forgetTarget()

            expect(currentTarget.value).toBeNull()
            expect(plateSolving.value).toEqual({inProgress: false, targetName: null, lastResult: null, stage: null})
            expect(pushToBlocked.value).toBeNull()
        })
    })

    describe('Push-To events', () => {
        it('keeps the camera a push direction was solved on', async () => {
            const {pushDirection} = useEventStream()

            await openWebSocket()
            await sendEvent({
                type: 'push_direction_updated', angle_deg: 37, distance_deg: 26.1,
                direction_hint: 'NE', is_close: false, fov_deg: 1.45, solved_camera: 'Neptune-C II',
            })

            expect(pushDirection.value).toEqual({
                angleDeg: 37, distanceDeg: 26.1, directionHint: 'NE', isClose: false,
                fovDeg: 1.45, solvedCamera: 'Neptune-C II',
            })
        })

        it('treats a push direction without a camera as unattributed', async () => {
            const {pushDirection} = useEventStream()

            await openWebSocket()
            await sendEvent({
                type: 'push_direction_updated', angle_deg: 90, distance_deg: 2,
                direction_hint: 'E', is_close: false, fov_deg: null,
            })

            expect(pushDirection.value.solvedCamera).toBeNull()
            expect(pushDirection.value.fovDeg).toBe(0)
        })

        it('updates plateSolving on plate_solving_started event', async () => {
            const {plateSolving} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'plate_solving_started', target_name: 'M31'})

            expect(plateSolving.value.inProgress).toBe(true)
            expect(plateSolving.value.targetName).toBe('M31')
        })

        it('counts the steps of a multi-attempt solve without naming them', async () => {
            // A cold solve works down several strategies, the last of which can run for
            // a minute. The status line has to move, or a slow solve reads as a hang --
            // but the counter alone does that. The strategy's own label is a diagnostic
            // long enough to push the object name off the bar, so it stays in the log.
            const {plateSolving, solvingMessage} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'plate_solving_started', target_name: 'M31'})
            await sendEvent({
                type: 'plate_solving_progress',
                stage: 'last solve (this session) + last known pointing + selected target',
                attempt: 2,
                total: 4,
            })

            expect(plateSolving.value.inProgress).toBe(true)
            expect(plateSolving.value.targetName).toBe('M31')
            expect(solvingMessage.value).toBe('Searching (step 2/4): M31')
            // The label is still carried, for anything that wants the detail.
            expect(plateSolving.value.stage.label).toContain('last known pointing')
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

            expect(solvingMessage.value).toBe('Searching: M31')
        })

        it('drops the step counter once the solve finishes', async () => {
            const {plateSolving, solvingMessage} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'plate_solving_started', target_name: 'M31'})
            await sendEvent({
                type: 'plate_solving_progress', stage: 'selected target', attempt: 2, total: 3,
            })
            await sendEvent({type: 'position_solved'})

            expect(plateSolving.value.stage).toBe(null)
            expect(solvingMessage.value).toBe('Found: M31')
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
            // "Found: M31" the whole way, because a solve ending was the only thing
            // that ever wrote to it. The movement states are now reported, and being
            // newer than the last verdict they have to win.
            const {solvingMessage} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'plate_solving_started', target_name: 'M31'})
            await sendEvent({type: 'position_solved'})
            expect(solvingMessage.value).toBe('Found: M31')

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
            expect(solvingMessage.value).toBe('Failed to find: M31')

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

            expect(solvingMessage.value).toBe('Found: M31')
        })

        it('keeps showing the search while a solve is running', async () => {
            // A blocker left over from before the solve started must not displace the
            // one thing the user most wants to see.
            const {solvingMessage} = useEventStream()

            await openWebSocket()
            await sendEvent({type: 'push_to_blocked', reason: 'Telescope is moving'})
            await sendEvent({type: 'plate_solving_started', target_name: 'M31'})

            expect(solvingMessage.value).toBe('Searching: M31')
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
