import {mount} from '@vue/test-utils'
import InstallProgressBar from './InstallProgressBar.vue'
import BaseSpinner from './BaseSpinner.vue'

describe('InstallProgressBar', () => {
    it('stacks a large spinner, the text, the bar, the overall line and the hint in a dialog', () => {
        const wrapper = mount(InstallProgressBar, {
            props: {progressText: 'Downloading', progressPercent: 42.4, overallProgressText: 'Overall 10%', hint: 'Be patient'},
        })

        expect(wrapper.find('.progress-icon').exists()).toBe(true)
        expect(wrapper.find('.progress-bar').classes()).toContain('progress-bar-lg')
        expect(wrapper.find('[role="progressbar"]').attributes('aria-valuenow')).toBe('42')
        expect(wrapper.find('.progress-fill').attributes('style')).toContain('width: 42.4%')
        expect(wrapper.find('.overall-progress-text').text()).toBe('Overall 10%')
        expect(wrapper.find('.progress-hint').text()).toBe('Be patient')
    })

    it('puts a small spinner beside the text when compact', () => {
        const wrapper = mount(InstallProgressBar, {props: {compact: true, progressText: 'Starting download...'}})

        expect(wrapper.find('.progress-icon').exists()).toBe(false)
        expect(wrapper.find('.progress-text').findComponent(BaseSpinner).props('size')).toBe('sm')
        expect(wrapper.find('.progress-text').text()).toBe('Starting download...')
        expect(wrapper.find('.progress-bar').exists()).toBe(false)
    })

    it('shows no spinner for a host that has its own', () => {
        const wrapper = mount(InstallProgressBar, {props: {compact: true, spinner: false, progressPercent: 0}})

        expect(wrapper.findComponent(BaseSpinner).exists()).toBe(false)
        expect(wrapper.find('.progress-text').exists()).toBe(false)
        expect(wrapper.find('.progress-bar').classes()).not.toContain('progress-bar-lg')
    })
})
