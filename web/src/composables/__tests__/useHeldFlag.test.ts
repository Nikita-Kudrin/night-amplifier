import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {mount} from '@vue/test-utils'
import {defineComponent, h, nextTick, ref} from 'vue'
import type {Ref} from 'vue'
import {useHeldFlag} from '../useHeldFlag'

const HOLD_MS = 1000

/** `useHeldFlag` inside a component, so its hold belongs to something that can unmount. */
function mountHeld(source: Ref<boolean>) {
    let held: Readonly<Ref<boolean>> | undefined
    const wrapper = mount(defineComponent({
        setup() {
            held = useHeldFlag(source, HOLD_MS)
            return () => h('div')
        },
    }))
    if (!held) throw new Error('setup did not run')
    return {wrapper, held}
}

describe('useHeldFlag', () => {
    beforeEach(() => vi.useFakeTimers())
    afterEach(() => vi.useRealTimers())

    it('starts as its source is, without a hold', () => {
        expect(mountHeld(ref(true)).held.value).toBe(true)
        expect(mountHeld(ref(false)).held.value).toBe(false)
    })

    it('rises at once', async () => {
        const source = ref(false)
        const {held} = mountHeld(source)

        source.value = true
        await nextTick()

        expect(held.value).toBe(true)
    })

    it('holds for the hold after its source falls, then falls', async () => {
        const source = ref(true)
        const {held} = mountHeld(source)

        source.value = false
        await nextTick()
        vi.advanceTimersByTime(HOLD_MS - 1)
        expect(held.value).toBe(true)

        vi.advanceTimersByTime(1)
        expect(held.value).toBe(false)
    })

    it('stays up through a rise during the hold, and holds again from the next fall', async () => {
        const source = ref(true)
        const {held} = mountHeld(source)

        source.value = false
        await nextTick()
        vi.advanceTimersByTime(HOLD_MS / 2)
        source.value = true
        await nextTick()
        vi.advanceTimersByTime(HOLD_MS)
        expect(held.value).toBe(true)

        source.value = false
        await nextTick()
        vi.advanceTimersByTime(HOLD_MS - 1)
        expect(held.value).toBe(true)
        vi.advanceTimersByTime(1)
        expect(held.value).toBe(false)
    })

    it('drops a pending hold with its component', async () => {
        const source = ref(true)
        const {wrapper} = mountHeld(source)

        source.value = false
        await nextTick()
        wrapper.unmount()

        expect(vi.getTimerCount()).toBe(0)
    })
})
