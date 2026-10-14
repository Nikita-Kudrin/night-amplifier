import {defineComponent, effectScope, h} from 'vue'
import {mount} from '@vue/test-utils'
import {tryOnBeforeUnmount, tryOnMounted, tryOnScopeDispose} from './lifecycle'

describe('lifecycle hooks for composables', () => {
    it('register as Vue’s own inside a component', () => {
        const calls: string[] = []
        const wrapper = mount(defineComponent({
            setup() {
                tryOnMounted(() => calls.push('mounted'))
                tryOnBeforeUnmount(() => calls.push('before unmount'))
                tryOnScopeDispose(() => calls.push('disposed'))
                return () => h('div')
            },
        }))
        expect(calls).toEqual(['mounted'])

        wrapper.unmount()
        expect(calls).toEqual(['mounted', 'before unmount', 'disposed'])
    })

    it('dispose with an effect scope, which has no mount', () => {
        const disposed = vi.fn()
        const mounted = vi.fn()
        const scope = effectScope()
        scope.run(() => {
            tryOnScopeDispose(disposed)
            tryOnMounted(mounted)
        })

        scope.stop()
        expect(disposed).toHaveBeenCalledOnce()
        expect(mounted).not.toHaveBeenCalled()
    })

    // Vue's own hooks warn outside a setup; a bare test call or a module store has none.
    it('do nothing, and say nothing, outside one', () => {
        const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
        const fn = vi.fn()

        tryOnMounted(fn)
        tryOnBeforeUnmount(fn)
        tryOnScopeDispose(fn)

        expect(fn).not.toHaveBeenCalled()
        expect(warn).not.toHaveBeenCalled()
        warn.mockRestore()
    })
})
