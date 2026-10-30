import {describe, it, expect, beforeEach, afterEach, vi} from 'vitest'
import {effectScope, nextTick} from 'vue'
import {
    MockWebSocket,
    createTestContext,
    cleanupTestContext,
    getWebSocket,
    openWebSocket,
    sendEvent,
    syncWebSocket,
    suppressConsoleErrors,
    setupGlobalWebSocketMock,
} from './webSocketTestUtils'

setupGlobalWebSocketMock()

vi.mock('../api', async (importOriginal) =>
    (await import('../../test/mockApi')).mockApi(await importOriginal()))

import {useEventStream as originalUseEventStream} from '../useEventStream'
import type {EventStream, Sync} from '../useEventStream'
import {catalogEntry} from '../../test/fixtures'
import {serverEvent} from '../../test/eventStream'
import type {Fields} from '../../test/eventStream'
import {flushPromises, mount} from '@vue/test-utils'
import type {VueWrapper} from '@vue/test-utils'

let currentApp: VueWrapper | null = null
function useEventStream(): EventStream {
    let result: EventStream | undefined
    currentApp = mount({
        setup() {
            result = originalUseEventStream()
            return () => {}
        }
    })
    return result!
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
            await sendEvent({type: 'camera_reconnecting', name: 'Guide', role: 'guide', attempt: 3, of: 31, next_attempt_in_s: 5})
            await sendEvent({type: 'camera_phase_changed', name: 'Guide', role: 'guide', phase: 'recovering'})
            await sendEvent({type: 'camera_phase_changed', name: 'Imaging', role: 'main', phase: 'idle'})
            expect(unresponsiveWarning.value).toContain('Reconnecting to Guide')

            await sendEvent({type: 'camera_phase_changed', name: 'Guide', role: 'guide', phase: 'guiding'})
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
                role: 'main',
                attempt: 2,
                of: 5,
                next_attempt_in_s: 10,
            })
            expect(unresponsiveWarning.value).toBe('Reconnecting to TestCam — attempt 2 of 5.')

            await sendEvent({
                type: 'camera_reconnect_failed',
                name: 'TestCam',
                role: 'main',
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

        // `null`, a number or an object without a `type` is JSON, not an event: it reaches no
        // handler, and the stream goes on.
        it('drops JSON that is not an event', async () => {
            const stream = useEventStream()
            const consoleSpy = suppressConsoleErrors()

            await openWebSocket()
            for (const text of ['null', '42', '"synced"', '{"state":"Capturing"}']) {
                expect(() => getWebSocket().simulateMessage(text)).not.toThrow()
            }
            await sendEvent({type: 'state_changed', state: 'Capturing'})

            expect(stream.captureState.value).toBe('Capturing')
            expect(consoleSpy).not.toHaveBeenCalled()
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

        // The drop count needs its denominator.
        it('counts dropped frames against the frames delivered', async () => {
            const {droppedCount, deliveredCount} = useEventStream()
            await openWebSocket()

            await sendEvent({type: 'frame_dropped', dropped_count: 3, delivered_count: 120})
            expect([droppedCount.value, deliveredCount.value]).toEqual([3, 120])
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
            const phases: string[] = []
            on('camera_phase_changed', (event) => phases.push(event.phase))

            await openWebSocket()
            getWebSocket().simulateMessage(JSON.stringify({type: 'camera_phase_changed', name: 'A', phase: 'idle'}))
            getWebSocket().simulateMessage(JSON.stringify({type: 'camera_phase_changed', name: 'A', phase: 'capturing'}))

            expect(phases).toEqual(['idle', 'capturing'])
        })

        it('runs the stream\'s own table before later subscribers, and survives one that throws', async () => {
            const {on, captureState} = useEventStream()
            const seen: string[] = []
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

        it('logs a handler whose promise rejects, as one that throws', async () => {
            const {on, captureState} = useEventStream()
            const seen: string[] = []
            const consoleSpy = suppressConsoleErrors()
            on('state_changed', async () => {
                throw new Error('async subscriber')
            })
            on('state_changed', () => seen.push(captureState.value))

            await openWebSocket()
            await sendEvent({type: 'state_changed', state: 'Capturing'})
            await flushPromises()

            expect(seen).toEqual(['Capturing'])
            expect(consoleSpy).toHaveBeenCalledWith('Handler for state_changed failed:', expect.objectContaining({message: 'async subscriber'}))
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

    describe('syncs', () => {
        it('hands each sync its cause and drop count, marking only the stream\'s first', async () => {
            const {onSync} = useEventStream()
            const syncs: Sync[] = []
            onSync((sync) => syncs.push(sync))

            await openWebSocket()
            await syncWebSocket()
            await syncWebSocket({cause: 'lagged', dropped: 144})
            getWebSocket().simulateClose()
            vi.advanceTimersByTime(1000)
            MockWebSocket.latest.simulateOpen()
            await syncWebSocket()

            expect(syncs).toEqual([
                {cause: 'connect', dropped: 0, first: true},
                {cause: 'lagged', dropped: 144, first: false},
                {cause: 'connect', dropped: 0, first: false},
            ])
        })

        it('stops calling a subscriber once unsubscribed, or once its scope is gone', async () => {
            const {onSync} = useEventStream()
            const early = vi.fn()
            const scoped = vi.fn()
            const off = onSync(early)
            const scope = effectScope()
            scope.run(() => onSync(scoped))

            await openWebSocket()
            off()
            scope.stop()
            await syncWebSocket()

            expect(early).not.toHaveBeenCalled()
            expect(scoped).not.toHaveBeenCalled()
        })

        // A lag is the server's business and the sync its whole answer: no toast.
        it('notes a lag in the debug log only', async () => {
            const debug = vi.spyOn(console, 'debug').mockImplementation(() => {})
            const {lastError} = useEventStream()

            await openWebSocket()
            await syncWebSocket({cause: 'lagged', dropped: 144})

            expect(debug).toHaveBeenCalledWith(expect.stringContaining('144'))
            expect(lastError.value).toBeNull()
            debug.mockRestore()
        })
    })

    describe('ASTAP installation events', () => {
        const astapEventTestCases = [
            {
                name: 'astap_install_starting',
                event: serverEvent('astap_install_starting', {component: 'D80 Database'}),
                expected: {component: 'D80 Database', stage: 'starting'},
            },
            {
                name: 'astap_install_progress',
                event: serverEvent('astap_install_progress', {
                    component: 'D80 Database',
                    bytes_downloaded: 52428800,
                    total_bytes: 1261887242,
                    percent: 4.15,
                    stage_id: 'downloading_database',
                    overall_percent: 52.08,
                }),
                expected: {
                    component: 'D80 Database',
                    stage: 'downloading',
                    percent: 4.15,
                    bytesDownloaded: 52428800,
                    totalBytes: 1261887242,
                    stageId: 'downloading_database',
                    overallPercent: 52.08,
                },
            },
            {
                name: 'astap_install_extracting',
                event: serverEvent('astap_install_extracting', {
                    component: 'D80 Database',
                    progress: 45.5,
                    stage_id: 'extracting_database',
                    overall_percent: 72.75,
                }),
                expected: {
                    component: 'D80 Database',
                    stage: 'extracting',
                    percent: 45.5,
                    stageId: 'extracting_database',
                    overallPercent: 72.75,
                },
            },
            {
                name: 'astap_install_completed',
                event: serverEvent('astap_install_completed', {
                    component: 'D80 Database',
                    stage_id: 'database_completed',
                    overall_percent: 100,
                }),
                expected: {
                    component: 'D80 Database',
                    stage: 'completed',
                    stageId: 'database_completed',
                    overallPercent: 100,
                },
            },
            {
                name: 'astap_install_failed',
                event: serverEvent('astap_install_failed', {
                    component: 'D80 Database',
                    error: 'Download timeout',
                }),
                expected: {component: 'D80 Database', stage: 'failed', error: 'Download timeout'},
            },
        ]

        it.each(astapEventTestCases)(
            'updates astapInstallProgress on $name event',
            async ({event, expected}) => {
                const {astapInstallProgress} = useEventStream()

                await openWebSocket()
                await sendEvent(event)

                expect(astapInstallProgress.value).toMatchObject(expected)
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
                event: serverEvent('catalog_install_starting'),
                expected: {stage: 'starting', fileName: '', percent: null, bytesDownloaded: 0, totalBytes: null, error: null},
            },
            {
                event: serverEvent('catalog_install_progress', {file_name: 'NGC.csv', percent: 50, bytes_downloaded: 512, total_bytes: 1024}),
                expected: {stage: 'downloading', fileName: 'NGC.csv', percent: 50, bytesDownloaded: 512, totalBytes: 1024, error: null},
            },
            {
                event: serverEvent('catalog_install_completed', {object_count: 13957}),
                expected: {stage: 'completed', objectCount: 13957, error: null},
            },
            {
                event: serverEvent('catalog_install_failed', {error: 'disk full'}),
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

    describe('Push-To snapshot', () => {
        /** The Push-To part of a sync snapshot, in the server's topic order. */
        async function sendSnapshot({target, direction, solve, blocker = null}: {
            target?: Fields<'target_changed'>
            direction?: Fields<'push_direction_updated'>
            solve?: Fields<'plate_solving_status'>
            blocker?: string | null
        }) {
            await sendEvent(target ? {type: 'target_changed', ...target} : {type: 'target_cleared'})
            await sendEvent(direction ? {type: 'push_direction_updated', ...direction} : {type: 'push_direction_cleared'})
            await sendEvent({type: 'plate_solving_status', in_progress: false, target_name: null, stage: null, last_result: null, ...solve})
            await sendEvent({type: 'push_to_blocked', reason: blocker})
            await syncWebSocket()
        }

        it('restores the target, direction, solve and blocker the server holds', async () => {
            const {currentTarget, pushDirection, plateSolving, pushToBlocked, solvingMessage} = useEventStream()

            await openWebSocket()
            await sendSnapshot({
                target: {name: 'Andromeda', designation: 'M31', ra_degrees: 10.68, dec_degrees: 41.27},
                direction: {angle_deg: 120, distance_deg: 4.5, direction_hint: 'SE', is_close: false, fov_deg: 1.45, solved_camera: 'Main'},
                solve: {in_progress: true, target_name: 'M31', stage: {stage: 'blind', attempt: 3, total: 4}},
                blocker: 'Telescope is moving',
            })

            expect(currentTarget.value).toEqual({name: 'Andromeda', designation: 'M31', ra_degrees: 10.68, dec_degrees: 41.27})
            expect(pushDirection.value).toEqual({
                angleDeg: 120, distanceDeg: 4.5, directionHint: 'SE', isClose: false,
                fovDeg: 1.45, solvedCamera: 'Main', stale: false,
            })
            expect(plateSolving.value).toEqual({
                inProgress: true, targetName: 'M31', lastResult: null,
                stage: {label: 'blind', attempt: 3, total: 4},
            })
            expect(pushToBlocked.value).toBe('Telescope is moving')
            expect(solvingMessage.value).toBe('Searching (step 3/4): M31')
        })

        // A failed solve leaves the last fix up, marked by the server: dismissing the
        // "Failed to find" chip clears this page's verdict and must leave the mark alone.
        it('keeps the server\'s stale mark on the arrow through a dismissed verdict', async () => {
            const {pushDirection, plateSolving, clearPlateSolving} = useEventStream()
            const pointing = {angle_deg: 120, distance_deg: 4.5, direction_hint: 'SE', is_close: false, fov_deg: 1.45, solved_camera: 'Main'} as const

            await openWebSocket()
            await sendSnapshot({
                target: {name: 'Andromeda', designation: 'M31', ra_degrees: 10.68, dec_degrees: 41.27},
                direction: {...pointing, stale: true},
                solve: {target_name: 'M31', last_result: 'failed'},
            })
            expect(pushDirection.value?.stale).toBe(true)

            clearPlateSolving()
            expect(plateSolving.value.lastResult).toBeNull()
            expect(pushDirection.value?.stale).toBe(true)

            await sendEvent({type: 'position_solved', ra_degrees: 10.68, dec_degrees: 41.27, ra_string: '', dec_string: '', stars_detected: null, confidence: 1, rotation_deg: 0})
            await sendEvent({type: 'push_direction_updated', ...pointing, stale: false})
            expect(pushDirection.value?.stale).toBe(false)
        })

        // What a lost event left behind -- "Searching...", an arrow, a blocker -- goes.
        it('clears what the server no longer holds', async () => {
            const {currentTarget, pushDirection, plateSolving, pushToBlocked, solvingMessage} = useEventStream()
            await openWebSocket()
            await sendEvent({type: 'target_changed', designation: 'M31', ra_degrees: 10.68, dec_degrees: 41.27})
            await sendEvent({type: 'push_direction_updated', angle_deg: 7, distance_deg: 1, direction_hint: 'N', is_close: false})
            await sendEvent({type: 'plate_solving_started', target_name: 'M31'})
            await sendEvent({type: 'push_to_blocked', reason: 'Waiting'})

            await sendSnapshot({})

            expect(currentTarget.value).toBeNull()
            expect(pushDirection.value).toBeNull()
            expect(plateSolving.value).toEqual({inProgress: false, targetName: null, lastResult: null, stage: null})
            expect(pushToBlocked.value).toBeNull()
            expect(solvingMessage.value).toBeNull()
        })

        it('ends a solve whose verdict it missed', async () => {
            const {plateSolving, solvingMessage} = useEventStream()
            await openWebSocket()
            await sendEvent({type: 'plate_solving_started', target_name: 'M31'})
            await sendEvent({type: 'plate_solving_progress', stage: 'blind', attempt: 3, total: 4})

            await sendEvent({type: 'plate_solving_status', in_progress: false, target_name: 'M31', stage: null, last_result: 'success'})

            expect(plateSolving.value).toEqual({inProgress: false, targetName: 'M31', lastResult: 'success', stage: null})
            expect(solvingMessage.value).toBe('Found: M31')
        })

        it('takes the arrow down on push_direction_cleared', async () => {
            const {pushDirection} = useEventStream()
            await openWebSocket()
            await sendEvent({type: 'push_direction_updated', angle_deg: 7, distance_deg: 1, direction_hint: 'N', is_close: false})

            await sendEvent({type: 'push_direction_cleared'})
            await sendEvent({type: 'push_direction_cleared'})

            expect(pushDirection.value).toBeNull()
        })
    })

    describe('Push-To answers', () => {
        it('aims at a target the panel set, dropping the old direction', async () => {
            const {aimAt, currentTarget, pushDirection} = useEventStream()
            await openWebSocket()
            await sendEvent({type: 'push_direction_updated', angle_deg: 1, distance_deg: 2, direction_hint: 'N', is_close: false})

            const target = catalogEntry({designation: 'M42', ra_degrees: 83.8, dec_degrees: -5.4})
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
                fovDeg: 1.45, solvedCamera: 'Neptune-C II', stale: false,
            })
        })

        it('treats a push direction without a camera as unattributed', async () => {
            const {pushDirection} = useEventStream()

            await openWebSocket()
            await sendEvent({
                type: 'push_direction_updated', angle_deg: 90, distance_deg: 2,
                direction_hint: 'E', is_close: false, fov_deg: null, solved_camera: null,
            })

            expect(pushDirection.value?.solvedCamera).toBeNull()
            expect(pushDirection.value?.fovDeg).toBe(0)
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
            expect(plateSolving.value.stage?.label).toContain('last known pointing')
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
