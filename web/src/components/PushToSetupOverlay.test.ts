import {mount} from '@vue/test-utils'
import type {VueWrapper} from '@vue/test-utils'
import {defineComponent, h} from 'vue'
import PushToSetupOverlay from './PushToSetupOverlay.vue'

/** An installer that reports what the test tells it to. */
const fakeInstaller = (name: string) => defineComponent({
    name,
    emits: ['installed', 'state-change'],
    setup: () => () => h('div'),
})

function mountOverlay() {
    return mount(PushToSetupOverlay, {
        global: {
            stubs: {AstapInstaller: fakeInstaller('AstapInstaller'), CatalogInstaller: fakeInstaller('CatalogInstaller')},
        },
    })
}

const installers = (wrapper: VueWrapper) =>
    ['AstapInstaller', 'CatalogInstaller'].map((name) => wrapper.findComponent({name}))

describe('PushToSetupOverlay', () => {
    it('reports the setup done only once both parts are installed', async () => {
        const wrapper = mountOverlay()
        const [astap, catalog] = installers(wrapper)

        astap.vm.$emit('installed')
        await wrapper.vm.$nextTick()
        expect(wrapper.emitted('installed')).toBeUndefined()
        expect(wrapper.find('.actions .btn').text()).toBe('Cancel')

        catalog.vm.$emit('installed')
        await wrapper.vm.$nextTick()
        expect(wrapper.emitted('installed')).toHaveLength(1)
        expect(wrapper.find('.actions .btn').text()).toBe('Done')
    })

    it('cannot be closed while either part installs', async () => {
        const wrapper = mountOverlay()
        const [, catalog] = installers(wrapper)

        catalog.vm.$emit('state-change', true)
        await wrapper.vm.$nextTick()
        expect(wrapper.find('.actions .btn').exists()).toBe(false)
        expect(wrapper.findComponent({name: 'BaseModal'}).props('persistent')).toBe(true)

        catalog.vm.$emit('state-change', false)
        await wrapper.vm.$nextTick()
        await wrapper.find('.actions .btn').trigger('click')
        expect(wrapper.emitted('close')).toHaveLength(1)
    })
})
