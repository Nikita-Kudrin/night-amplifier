import {computed} from 'vue'
import {
    setTargetByName,
    setTargetByCoordinates,
    clearTarget as apiClearTarget,
    cancelPushToSolve as apiCancelSolve,
} from './api'
import {runDirectly} from './useError'
import type {ErrorHandler} from './useError'
import type {EventStream} from './useEventStream'

export interface PushToTargetOptions {
    withErrorHandling?: ErrorHandler
    /** The provided `useEventStream()` */
    eventStream: Pick<EventStream,
        'currentTarget' | 'pushDirection' | 'plateSolving' | 'aimAt' | 'forgetTarget' | 'clearPlateSolving'>
}

/**
 * Push-To target management over the event stream, which owns the target, the push
 * direction and the solve (`events/pushToEvents.ts`): requests go to the server here, and
 * their answers are handed to the stream rather than written into its refs.
 * `withErrorHandling` wraps each request; errors propagate without one.
 */
export function usePushToTarget({withErrorHandling = runDirectly, eventStream}: PushToTargetOptions) {
    const isSolving = computed(() => eventStream.plateSolving.value.inProgress)

    function selectTargetByName(designation: string) {
        return withErrorHandling(async () => eventStream.aimAt(await setTargetByName(designation)))
    }

    function selectTargetByCoordinates(ra: number, dec: number) {
        return withErrorHandling(async () => eventStream.aimAt(await setTargetByCoordinates(ra, dec)))
    }

    function clearTarget() {
        return withErrorHandling(async () => {
            await apiClearTarget()
            eventStream.forgetTarget()
        })
    }

    function cancelSolve() {
        return withErrorHandling(async () => {
            await apiCancelSolve()
            // The server also broadcasts `plate_solving_cancelled`, but clearing here
            // keeps the button from lingering for a round trip.
            eventStream.clearPlateSolving()
        })
    }

    return {
        currentTarget: eventStream.currentTarget,
        pushDirection: eventStream.pushDirection,
        isSolving,
        selectTargetByName,
        selectTargetByCoordinates,
        clearTarget,
        cancelSolve,
    }
}
