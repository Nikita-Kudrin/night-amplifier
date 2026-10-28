import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {mount} from '@vue/test-utils'
import {defineComponent, h, nextTick, ref} from 'vue'
import type {Ref} from 'vue'
import {useSolveIndicator} from '../useSolveIndicator'
import {NOT_SOLVING} from '../events/pushToEvents'
import type {PlateSolving} from '../events/pushToEvents'
import {FAILED_SOLVE_HOLD_MS, SOLVING_HOLD_MS} from '../../constants'

const SOLVING: PlateSolving = {...NOT_SOLVING, inProgress: true, targetName: 'M31'}
const ended = (lastResult: PlateSolving['lastResult']): PlateSolving => ({...NOT_SOLVING, targetName: 'M31', lastResult})

/** The indicator inside a component, so its hold belongs to something that can unmount. */
function mountIndicator(plateSolving: Ref<PlateSolving | null>) {
    let indicator: ReturnType<typeof useSolveIndicator> | undefined
    mount(defineComponent({
        setup() {
            indicator = useSolveIndicator(plateSolving)
            return () => h('div')
        },
    }))
    if (!indicator) throw new Error('setup did not run')
    return indicator
}

/** How long `solving` outlasts a solve that ended with `result`. */
async function holdAfter(result: PlateSolving['lastResult']) {
    const plateSolving = ref<PlateSolving | null>(SOLVING)
    const {solving} = mountIndicator(plateSolving)
    plateSolving.value = ended(result)
    await nextTick()
    let held = 0
    while (solving.value) {
        vi.advanceTimersByTime(100)
        held += 100
    }
    return held
}

describe('useSolveIndicator', () => {
    beforeEach(() => vi.useFakeTimers())
    afterEach(() => vi.useRealTimers())

    it('holds the compass one turn after a solve that ended otherwise', async () => {
        expect(await holdAfter('success')).toBe(SOLVING_HOLD_MS)
        expect(await holdAfter('cancelled')).toBe(SOLVING_HOLD_MS)
    })

    // Pro retries an unusable frame 2 s after it failed: a 1 s hold flashed the readout between.
    it('holds it past the next attempt after a failed one', async () => {
        expect(await holdAfter('failed')).toBe(FAILED_SOLVE_HOLD_MS)
        expect(FAILED_SOLVE_HOLD_MS).toBeGreaterThan(2000)
    })

    it('hands over the solve\'s step, and nothing without a solve', () => {
        const stage = {label: 'blind', attempt: 2, total: 3}
        const plateSolving = ref<PlateSolving | null>({...SOLVING, stage})
        const indicator = mountIndicator(plateSolving)
        expect(indicator.stage.value).toEqual(stage)

        plateSolving.value = null
        expect(indicator.stage.value).toBeNull()
    })
})
