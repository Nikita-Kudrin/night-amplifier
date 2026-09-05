import {describe, it, expect, afterEach} from 'vitest'
import {mount} from '@vue/test-utils'
import BaseSplitButton from './BaseSplitButton.vue'

let wrappers = []

function mountButton(props = {}, slots = {}) {
    const wrapper = mount(BaseSplitButton, {
        props: {label: 'Connect', options: [{value: 'guide', label: 'Connect as guide'}], ...props},
        slots,
        attachTo: document.body,
    })
    wrappers.push(wrapper)
    return wrapper
}

describe('BaseSplitButton', () => {
    // The menu is teleported to `body` and outlives its wrapper, so a leaked one
    // is what the *next* test's `document.querySelector` would find and click.
    afterEach(() => {
        wrappers.forEach((wrapper) => wrapper.unmount())
        wrappers = []
    })

    it('renders the label as the primary action text', () => {
        const wrapper = mountButton()

        expect(wrapper.find('.split-button-main').text()).toBe('Connect')
    })

    it('emits click from the primary action', async () => {
        const wrapper = mountButton()

        await wrapper.find('.split-button-main').trigger('click')

        expect(wrapper.emitted('click')).toHaveLength(1)
    })

    it('hides the menu trigger when there is nothing in the menu', () => {
        const wrapper = mountButton({options: []})

        expect(wrapper.find('.split-button-toggle').exists()).toBe(false)
    })

    it('emits the chosen option value', async () => {
        const wrapper = mountButton()

        await wrapper.find('.split-button-toggle').trigger('click')
        document.querySelector('.split-button-item').click()

        expect(wrapper.emitted('select')[0]).toEqual(['guide'])
    })

    describe('icon-only form', () => {
        it('shows the slot instead of the label', () => {
            const wrapper = mountButton({label: 'Download'}, {default: '<svg class="icon" />'})

            const main = wrapper.find('.split-button-main')
            expect(main.text()).toBe('')
            expect(main.find('svg.icon').exists()).toBe(true)
        })

        // The label is the only thing naming an icon-only button, so it has to
        // survive as a tooltip as well as an accessible name.
        it('keeps the label as a tooltip', () => {
            const wrapper = mountButton({label: 'Download'}, {default: '<svg class="icon" />'})

            const main = wrapper.find('.split-button-main')
            expect(main.attributes('title')).toBe('Download')
            expect(main.attributes('aria-label')).toBe('Download')
        })

        // A button already showing its text does not need a tooltip repeating it.
        it('adds no tooltip when the label is on screen', () => {
            const wrapper = mountButton()

            expect(wrapper.find('.split-button-main').attributes('title')).toBeUndefined()
        })
    })

    // A caller has to be able to tell an open menu from a closed one — a menu is
    // teleported to `body`, so it outlives anything hiding the button itself.
    describe('menuToggle', () => {
        it('reports the menu opening', async () => {
            const wrapper = mountButton()

            await wrapper.find('.split-button-toggle').trigger('click')

            expect(wrapper.emitted('menuToggle').at(-1)).toEqual([true])
        })

        it('reports the menu closing again', async () => {
            const wrapper = mountButton()

            await wrapper.find('.split-button-toggle').trigger('click')
            await wrapper.find('.split-button-toggle').trigger('click')

            expect(wrapper.emitted('menuToggle').at(-1)).toEqual([false])
        })

        it('reports the close that choosing an option causes', async () => {
            const wrapper = mountButton()
            await wrapper.find('.split-button-toggle').trigger('click')

            document.querySelector('.split-button-item').click()
            await wrapper.vm.$nextTick()

            expect(wrapper.emitted('menuToggle').at(-1)).toEqual([false])
        })

        it('reports the close that a click outside causes', async () => {
            const wrapper = mountButton()
            await wrapper.find('.split-button-toggle').trigger('click')

            document.body.click()
            await wrapper.vm.$nextTick()

            expect(wrapper.emitted('menuToggle').at(-1)).toEqual([false])
        })
    })

    describe('menu label', () => {
        it('names the menu trigger from the prop', () => {
            const wrapper = mountButton({menuLabel: 'More download options'})

            expect(wrapper.find('.split-button-toggle').attributes('aria-label')).toBe(
                'More download options'
            )
        })

        it('falls back to a generic name', () => {
            const wrapper = mountButton()

            expect(wrapper.find('.split-button-toggle').attributes('aria-label')).toBe(
                'More options'
            )
        })
    })
})
