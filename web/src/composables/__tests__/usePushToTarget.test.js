import {usePushToTarget} from '../usePushToTarget.js'
import {createEventStreamStub} from '../../test/eventStream.js'
import * as api from '../api.js'

vi.mock('../api.js', async (importOriginal) =>
    (await import('../../test/mockApi.js')).mockApi(await importOriginal()))

describe('usePushToTarget', () => {
    let eventStream

    beforeEach(() => {
        vi.clearAllMocks()
        eventStream = createEventStreamStub()
    })

    it('shows the target and direction the event stream holds', () => {
        eventStream.currentTarget.value = {designation: 'M31'}
        eventStream.pushDirection.value = {distanceDeg: 10}
        const {currentTarget, pushDirection} = usePushToTarget({eventStream})

        expect(currentTarget.value.designation).toBe('M31')
        expect(pushDirection.value.distanceDeg).toBe(10)

        eventStream.currentTarget.value = {designation: 'M42'}
        expect(currentTarget.value.designation).toBe('M42')
    })

    // The stream owns the target: the panel hands it the server's answer instead of
    // writing its refs, so a status read cannot race the panel's own writes.
    it('hands the target the server answers with to the stream', async () => {
        const target = {designation: 'M31', ra_degrees: 10, dec_degrees: 41}
        api.setTargetByName.mockResolvedValue(target)
        api.setTargetByCoordinates.mockResolvedValue({...target, designation: null})
        const {currentTarget, selectTargetByName, selectTargetByCoordinates} = usePushToTarget({eventStream})

        await expect(selectTargetByName('M31')).resolves.toEqual(target)
        expect(api.setTargetByName).toHaveBeenCalledWith('M31')
        expect(eventStream.aimAt).toHaveBeenCalledWith(target)
        expect(currentTarget.value).toEqual(target)

        await selectTargetByCoordinates(10, 41)
        expect(api.setTargetByCoordinates).toHaveBeenCalledWith(10, 41)
        expect(eventStream.aimAt).toHaveBeenLastCalledWith({...target, designation: null})
    })

    it('forgets the target only once the server cleared it', async () => {
        eventStream.currentTarget.value = {designation: 'M31'}
        api.clearTarget.mockRejectedValueOnce(new Error('offline'))
        const {currentTarget, clearTarget} = usePushToTarget({eventStream})

        await expect(clearTarget()).rejects.toThrow('offline')
        expect(currentTarget.value).toEqual({designation: 'M31'})

        await clearTarget()
        expect(eventStream.forgetTarget).toHaveBeenCalledOnce()
        expect(currentTarget.value).toBeNull()
    })

    describe('isSolving', () => {
        it('follows the event stream once a solve starts', () => {
            // Regression: `isSolving` was a plain ref written only by the mount-time
            // status poll and by cancelSolve(). Nothing ever set it true afterwards,
            // so the panel's spinner and its Cancel button never appeared no matter
            // how long a solve ran -- which is what "no indicator in the UI" meant.
            const {isSolving} = usePushToTarget({eventStream})
            expect(isSolving.value).toBe(false)

            eventStream.plateSolving.value = {inProgress: true, targetName: 'M31', lastResult: null, stage: null}
            expect(isSolving.value).toBe(true)

            eventStream.plateSolving.value = {inProgress: false, targetName: 'M31', lastResult: 'success', stage: null}
            expect(isSolving.value).toBe(false)
        })

        it('clears immediately on cancel rather than waiting for the round trip', async () => {
            eventStream.plateSolving.value = {inProgress: true, targetName: 'M31', lastResult: null, stage: null}
            const {isSolving, cancelSolve} = usePushToTarget({eventStream})

            expect(isSolving.value).toBe(true)
            await cancelSolve()

            expect(api.cancelPushToSolve).toHaveBeenCalled()
            expect(eventStream.clearPlateSolving).toHaveBeenCalled()
            expect(isSolving.value).toBe(false)
        })
    })

    it('reads no status of its own: the stream loads it when its socket opens', () => {
        usePushToTarget({eventStream})
        expect(api.getPushToStatus).not.toHaveBeenCalled()
    })
})
