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

import {useEventStream} from '../useWebSocket.js'

describe('useEventStream', () => {
    beforeEach(createTestContext)
    afterEach(cleanupTestContext)

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
    })
})
