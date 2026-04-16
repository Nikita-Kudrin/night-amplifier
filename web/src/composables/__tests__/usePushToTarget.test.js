import {ref} from 'vue'
import {usePushToTarget} from '../usePushToTarget.js'
import * as api from '../api.js'

vi.mock('../api.js', () => ({
    getPushToStatus: vi.fn(),
    setTargetByName: vi.fn(),
    setTargetByCoordinates: vi.fn(),
    clearTarget: vi.fn(),
}))

describe('usePushToTarget', () => {
    beforeEach(() => {
        vi.clearAllMocks()
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
})
