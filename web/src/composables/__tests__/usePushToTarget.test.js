import {ref} from 'vue'
import {usePushToTarget as originalUsePushToTarget} from '../usePushToTarget.js'
import { mount } from '@vue/test-utils'

let currentApp = null;
function usePushToTarget(...args) {
    let result;
    currentApp = mount({
        setup() {
            result = originalUsePushToTarget(...args)
            return () => {}
        }
    })
    return result
}
import * as api from '../api.js'

vi.mock('../api.js', () => ({
    getPushToStatus: vi.fn(),
    setTargetByName: vi.fn(),
    setTargetByCoordinates: vi.fn(),
    clearTarget: vi.fn(),
    cancelPushToSolve: vi.fn(),
}))

describe('usePushToTarget', () => {
    beforeEach(() => {
        vi.clearAllMocks()
    })

    afterEach(() => {
        if (currentApp) {
            currentApp.unmount()
            currentApp = null
        }
    })

    it('syncs with eventStream if provided', () => {
        const eventStream = {
            currentTarget: ref({designation: 'M31'}),
            pushDirection: ref({distance_deg: 10}),
        }
        const {currentTarget, pushDirection} = usePushToTarget({eventStream})

        expect(currentTarget.value.designation).toBe('M31')
        expect(pushDirection.value.distance_deg).toBe(10)

        // Update eventStream and check sync
        eventStream.currentTarget.value = {designation: 'M42'}
        expect(currentTarget.value.designation).toBe('M42')
    })

    it('uses local refs if no eventStream is provided', () => {
        const {currentTarget, pushDirection} = usePushToTarget()
        expect(currentTarget.value).toBe(null)
        expect(pushDirection.value).toBe(null)
    })

    it('correctly handles setTargetByName response', async () => {
        const mockTarget = {designation: 'M31', ra_degrees: 10, dec_degrees: 41}
        api.setTargetByName.mockResolvedValue(mockTarget)

        const {currentTarget, selectTargetByName} = usePushToTarget()
        await selectTargetByName('M31')

        expect(api.setTargetByName).toHaveBeenCalledWith('M31')
        expect(currentTarget.value).toEqual(mockTarget)
    })

    it('correctly handles refreshStatus with current_target', async () => {
        const mockStatus = {
            current_target: {designation: 'M31'},
            current_position: {ra_degrees: 10, dec_degrees: 40},
            direction: {distance_deg: 1},
        }
        api.getPushToStatus.mockResolvedValue(mockStatus)

        const {currentTarget, refreshStatus} = usePushToTarget()
        await refreshStatus()

        expect(currentTarget.value).toEqual(mockStatus.current_target)
    })

    describe('isSolving', () => {
        it('follows the event stream once a solve starts', async () => {
            // Regression: `isSolving` was a plain ref written only by the mount-time
            // status poll and by cancelSolve(). Nothing ever set it true afterwards,
            // so the panel's spinner and its Cancel button never appeared no matter
            // how long a solve ran -- which is what "no indicator in the UI" meant.
            api.getPushToStatus.mockResolvedValue({
                current_target: null, last_position: null, direction: null, is_solving: false,
            })

            const eventStream = {
                currentTarget: ref(null),
                pushDirection: ref(null),
                plateSolving: ref({inProgress: false, targetName: null, lastResult: null, stage: null}),
            }
            const {isSolving} = usePushToTarget({eventStream})

            expect(isSolving.value).toBe(false)

            eventStream.plateSolving.value = {
                inProgress: true, targetName: 'M31', lastResult: null, stage: null,
            }
            expect(isSolving.value).toBe(true)

            eventStream.plateSolving.value = {
                inProgress: false, targetName: 'M31', lastResult: 'success', stage: null,
            }
            expect(isSolving.value).toBe(false)
        })

        it('falls back to the status poll when there is no event stream', async () => {
            api.getPushToStatus.mockResolvedValue({
                current_target: null, last_position: null, direction: null, is_solving: true,
            })

            const {isSolving, refreshStatus} = usePushToTarget()
            await refreshStatus()

            expect(isSolving.value).toBe(true)
        })

        it('clears immediately on cancel rather than waiting for the round trip', async () => {
            api.cancelPushToSolve.mockResolvedValue({})
            api.getPushToStatus.mockResolvedValue({
                current_target: null, last_position: null, direction: null, is_solving: true,
            })

            const clearPlateSolving = vi.fn()
            const eventStream = {
                currentTarget: ref(null),
                pushDirection: ref(null),
                plateSolving: ref({inProgress: true, targetName: 'M31', lastResult: null, stage: null}),
                clearPlateSolving,
            }
            const {isSolving, cancelSolve} = usePushToTarget({eventStream})

            expect(isSolving.value).toBe(true)
            await cancelSolve()

            expect(api.cancelPushToSolve).toHaveBeenCalled()
            expect(clearPlateSolving).toHaveBeenCalled()
        })
    })
})
