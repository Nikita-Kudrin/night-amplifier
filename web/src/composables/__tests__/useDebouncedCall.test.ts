import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {mount} from '@vue/test-utils'
import {defineComponent, h} from 'vue'
import {useDebouncedCall} from '../useDebouncedCall'

describe('useDebouncedCall', () => {
    beforeEach(() => vi.useFakeTimers())
    afterEach(() => vi.useRealTimers())

    it('runs once with the last arguments, the delay after the last schedule', () => {
        const fn = vi.fn((_label: string, _n: number) => {})
        const {schedule} = useDebouncedCall(fn, 300)

        schedule('a', 1)
        vi.advanceTimersByTime(200)
        schedule('b', 2)
        vi.advanceTimersByTime(299)
        expect(fn).not.toHaveBeenCalled()

        vi.advanceTimersByTime(1)
        expect(fn).toHaveBeenCalledTimes(1)
        expect(fn).toHaveBeenCalledWith('b', 2)
    })

    it('flush runs the pending call now and only once', () => {
        const fn = vi.fn((_n: number) => 'done')
        const {schedule, flush} = useDebouncedCall(fn, 300)

        schedule(7)
        expect(flush()).toBe('done')
        vi.advanceTimersByTime(1000)

        expect(fn).toHaveBeenCalledTimes(1)
        expect(fn).toHaveBeenCalledWith(7)
    })

    it('flush with nothing pending does nothing', () => {
        const fn = vi.fn()
        const {schedule, flush} = useDebouncedCall(fn, 300)

        flush()
        schedule(1)
        vi.advanceTimersByTime(300)
        flush()

        expect(fn).toHaveBeenCalledTimes(1)
    })

    it('cancel drops the pending call', () => {
        const fn = vi.fn()
        const {schedule, cancel} = useDebouncedCall(fn, 300)

        schedule(1)
        cancel()
        vi.advanceTimersByTime(1000)

        expect(fn).not.toHaveBeenCalled()
    })

    // The owner's `emit` still works in beforeUnmount; after it, Vue drops the event.
    it('runs a pending call when its component unmounts, while it can still emit', () => {
        const onApply = vi.fn()
        let schedule: (value: number) => void = () => {}
        const Child = defineComponent({
            emits: ['apply'],
            setup(_, {emit}) {
                ;({schedule} = useDebouncedCall((value: number) => emit('apply', value), 300))
                return () => null
            },
        })
        const wrapper = mount({render: () => h(Child, {onApply})})

        schedule(42)
        wrapper.unmount()

        expect(onApply).toHaveBeenCalledWith(42)
        vi.advanceTimersByTime(1000)
        expect(onApply).toHaveBeenCalledTimes(1)
    })
})
