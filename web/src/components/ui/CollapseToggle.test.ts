import {mount} from '@vue/test-utils'
import CollapseToggle from './CollapseToggle.vue'

describe('CollapseToggle', () => {
    it('turns the chevron while folded and says whether it is open', () => {
        const wrapper = mount(CollapseToggle, {props: {modelValue: true}})

        expect(wrapper.find('svg').classes()).toContain('collapsed')
        expect(wrapper.attributes('aria-expanded')).toBe('false')
    })

    it('folds on its own click, which still reaches document', async () => {
        const onDocument = vi.fn()
        document.addEventListener('click', onDocument)
        const wrapper = mount({
            components: {CollapseToggle},
            data: () => ({collapsed: false}),
            template: '<CollapseToggle v-model="collapsed"/>',
        }, {attachTo: document.body})

        wrapper.find('button').element.click()
        await wrapper.vm.$nextTick()

        expect(wrapper.vm.collapsed).toBe(true)
        // Stopping it left `useClickOutside` popovers (Connect menu, help bubble) open over a folded panel.
        expect(onDocument).toHaveBeenCalledTimes(1)
        document.removeEventListener('click', onDocument)
        wrapper.unmount()
    })

    it('folds once per click inside a header that folds on click, bound one-way', async () => {
        const wrapper = mount({
            components: {CollapseToggle},
            data: () => ({collapsed: false}),
            template: '<div @click="collapsed = !collapsed"><CollapseToggle :model-value="collapsed"/></div>',
        })

        await wrapper.find('button').trigger('click')
        expect(wrapper.vm.collapsed).toBe(true)
        expect(wrapper.find('svg').classes()).toContain('collapsed')

        await wrapper.find('button').trigger('click')
        expect(wrapper.vm.collapsed).toBe(false)
        expect(wrapper.find('svg').classes()).not.toContain('collapsed')
    })
})
