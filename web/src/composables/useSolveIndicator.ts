import {computed, toValue} from 'vue'
import type {MaybeRefOrGetter} from 'vue'
import {FAILED_SOLVE_HOLD_MS, SOLVING_HOLD_MS} from '../constants'
import {useHeldFlag} from './useHeldFlag'
import type {PlateSolving} from './events/pushToEvents'

/**
 * What the Push-To chevron shows of the solve, the same on `/` and the eyepieces. `solving`
 * holds longer after a failure, since the next attempt follows in about 2 s; whether the
 * readout is stale travels with the direction (`ClientPushDirection::stale`).
 */
export function useSolveIndicator(plateSolving: MaybeRefOrGetter<PlateSolving | null>) {
    const lastResult = computed(() => toValue(plateSolving)?.lastResult ?? null)
    const solving = useHeldFlag(
        () => toValue(plateSolving)?.inProgress ?? false,
        () => (lastResult.value === 'failed' ? FAILED_SOLVE_HOLD_MS : SOLVING_HOLD_MS),
    )
    const stage = computed(() => toValue(plateSolving)?.stage ?? null)
    return {solving, stage}
}
