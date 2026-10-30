import {mount} from '@vue/test-utils'
import CompassIcon from './CompassIcon.vue'

describe('CompassIcon', () => {
    it('holds its needle still by default, as the header shows it', () => {
        const wrapper = mount(CompassIcon)

        expect(wrapper.find('circle').exists()).toBe(true)
        expect(wrapper.find('.needle').classes()).not.toContain('spinning')
    })

    it('turns only the needle while spinning, never the ring', () => {
        const wrapper = mount(CompassIcon, {props: {spinning: true}})

        expect(wrapper.find('.needle').classes()).toContain('spinning')
        expect(wrapper.classes()).not.toContain('spinning')
        expect(wrapper.find('circle').classes()).not.toContain('spinning')
    })
})
