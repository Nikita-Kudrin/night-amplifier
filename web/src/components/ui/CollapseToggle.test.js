import {mount} from '@vue/test-utils'
import CollapseToggle from './CollapseToggle.vue'

describe('CollapseToggle', () => {
    it('turns the chevron while folded and says whether it is open', () => {
        const wrapper = mount(CollapseToggle, {props: {modelValue: true}})

        expect(wrapper.find('svg').classes()).toContain('collapsed')
        expect(wrapper.attributes('aria-expanded')).toBe('false')
    })

    it('folds once per click, even inside a header that toggles on click', async () => {
        const onHeader = vi.fn()
        const wrapper = mount({
            components: {CollapseToggle},
            data: () => ({collapsed: false}),
            methods: {onHeader},
            template: '<div @click="onHeader"><CollapseToggle v-model="collapsed"/></div>',
        })

        await wrapper.find('button').trigger('click')

        expect(wrapper.vm.collapsed).toBe(true)
        expect(onHeader).not.toHaveBeenCalled()
    })
})
