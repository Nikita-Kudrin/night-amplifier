import {usePushToTarget} from '../usePushToTarget'
import {createEventStreamStub} from '../../test/eventStream'
import type {EventStream} from '../useEventStream'
import {catalogEntry} from '../../test/fixtures'
import type {Coordinate} from '../api.types'
import * as api from '../api'

vi.mock('../api', async (importOriginal) =>
    (await import('../../test/mockApi')).mockApi(await importOriginal()))

describe('usePushToTarget', () => {
    let eventStream: EventStream

    beforeEach(() => {
        vi.clearAllMocks()
        eventStream = createEventStreamStub()
    })

    it('shows the target and direction the event stream holds', () => {
        eventStream.currentTarget.value = catalogEntry({designation: 'M31'})
        eventStream.pushDirection.value = {
            angleDeg: 0, distanceDeg: 10, directionHint: 'N', isClose: false, fovDeg: 1, solvedCamera: null, stale: false,
        }
        const {currentTarget, pushDirection} = usePushToTarget({eventStream})

        expect(currentTarget.value).toMatchObject({designation: 'M31'})
        expect(pushDirection.value?.distanceDeg).toBe(10)

        eventStream.currentTarget.value = catalogEntry({designation: 'M42'})
        expect(currentTarget.value).toMatchObject({designation: 'M42'})
    })

    // The stream owns the target: the panel hands it the server's answer instead of
    // writing its refs, so a status read cannot race the panel's own writes.
    it('hands the target the server answers with to the stream', async () => {
        const target = catalogEntry({designation: 'M31', ra_degrees: 10, dec_degrees: 41})
        const coordinates: Coordinate = {ra_degrees: 10, dec_degrees: 41, ra_string: '00h40m', dec_string: '+41°'}
        vi.mocked(api.setTargetByName).mockResolvedValue(target)
        vi.mocked(api.setTargetByCoordinates).mockResolvedValue(coordinates)
        const {currentTarget, selectTargetByName, selectTargetByCoordinates} = usePushToTarget({eventStream})

        await expect(selectTargetByName('M31')).resolves.toEqual(target)
        expect(api.setTargetByName).toHaveBeenCalledWith('M31')
        expect(eventStream.aimAt).toHaveBeenCalledWith(target)
        expect(currentTarget.value).toEqual(target)

        await selectTargetByCoordinates(10, 41)
        expect(api.setTargetByCoordinates).toHaveBeenCalledWith(10, 41)
        expect(eventStream.aimAt).toHaveBeenLastCalledWith(coordinates)
    })

    it('forgets the target only once the server cleared it', async () => {
        const target = catalogEntry({designation: 'M31'})
        eventStream.currentTarget.value = target
        vi.mocked(api.clearTarget).mockRejectedValueOnce(new Error('offline'))
        const {currentTarget, clearTarget} = usePushToTarget({eventStream})

        await expect(clearTarget()).rejects.toThrow('offline')
        expect(currentTarget.value).toEqual(target)

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

    it('takes the stream it hands answers to', () => {
        // @ts-expect-error without a stream there is nothing to own the target
        expect(() => usePushToTarget({})).toThrow()
    })
})
