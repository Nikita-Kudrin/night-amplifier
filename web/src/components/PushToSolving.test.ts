import {describe, it, expect} from 'vitest'
import {h} from 'vue'
import {mount} from '@vue/test-utils'
import type {VueWrapper} from '@vue/test-utils'
import PushToSolving from './PushToSolving.vue'

/** Each segment's state, in order: `done`, `current` or `empty`. */
function segments(wrapper: VueWrapper) {
    return wrapper.findAll('.solve-progress .segment')
        .map(segment => segment.classes('done') ? 'done' : segment.classes('current') ? 'current' : 'empty')
}

const stage = (attempt: number, total: number) => ({label: 'blind', attempt, total})

describe('PushToSolving', () => {
    it('draws one bar per step, the tried ones done, and no visible step counter', () => {
        const wrapper = mount(PushToSolving, {props: {stage: stage(2, 3)}})

        expect(segments(wrapper)).toEqual(['done', 'current', 'empty'])
        expect(wrapper.find('.segments').attributes('aria-hidden')).toBe('true')
        expect(wrapper.find('.segments').text()).toBe('')
        expect(wrapper.find('.solve-progress .visually-hidden').text()).toBe('step 2 of 3')
    })

    it.each([
        [1, 4, ['current', 'empty', 'empty', 'empty']],
        [4, 4, ['done', 'done', 'done', 'current']],
        [3, 6, ['done', 'done', 'current', 'empty', 'empty', 'empty']],
    ])('marks step %i of %i', (attempt, total, expected) => {
        expect(segments(mount(PushToSolving, {props: {stage: stage(attempt, total)}}))).toEqual(expected)
    })

    it('draws no bar for a single-step solve or before the first progress, only the compass', () => {
        for (const wrapper of [mount(PushToSolving, {props: {stage: stage(1, 1)}}), mount(PushToSolving)]) {
            expect(wrapper.find('.solve-progress').exists()).toBe(false)
            expect(wrapper.find('.compass-icon').exists()).toBe(true)
            expect(wrapper.text()).toBe('')
        }
    })

    // The compass would move by the bar's height when the first step arrived.
    it('keeps the bar\'s row laid out before the first step', async () => {
        const wrapper = mount(PushToSolving)
        expect(wrapper.find('.solving .progress-row').exists()).toBe(true)

        await wrapper.setProps({stage: stage(1, 3)})
        expect(wrapper.find('.solving .progress-row .solve-progress').exists()).toBe(true)
    })

    // A live region announces text changes, not attribute ones: the step must be its text.
    it('announces each step as the live region\'s text', async () => {
        const wrapper = mount(PushToSolving, {props: {stage: stage(2, 3)}})
        const status = () => wrapper.find('[role="status"]')

        expect(status().attributes('aria-label')).toBe('Plate solving')
        expect(status().text()).toBe('step 2 of 3')

        await wrapper.setProps({stage: stage(3, 3)})
        expect(status().text()).toBe('step 3 of 3')
    })

    it('never fills more bars than the plan has', () => {
        expect(segments(mount(PushToSolving, {props: {stage: stage(5, 3)}}))).toEqual(['done', 'done', 'current'])
        expect(segments(mount(PushToSolving, {props: {stage: stage(0, 3)}}))).toEqual(['current', 'empty', 'empty'])
    })

    it('moves on with the next progress event', async () => {
        const wrapper = mount(PushToSolving, {props: {stage: stage(2, 3)}})

        await wrapper.setProps({stage: stage(3, 3)})

        expect(segments(wrapper)).toEqual(['done', 'done', 'current'])
    })

    // One grid cell holds both, so the chip takes the readout's size and the bar stretches across it.
    it('lays what it stands in for unseen beside the compass, and announces only the step', () => {
        const wrapper = mount(PushToSolving, {
            props: {stage: stage(2, 3)},
            slots: {default: () => h('span', {class: 'readout'}, '12.3°')},
        })
        const standIn = wrapper.find('.push-to-solving > .stand-in')

        expect(standIn.attributes('aria-hidden')).toBe('true')
        expect(standIn.find('.readout').text()).toBe('12.3°')
        expect(wrapper.find('.push-to-solving > .solving .solve-progress').exists()).toBe(true)
        expect(wrapper.find('.solving').text()).toBe('step 2 of 3')
    })

    it('has no stand-in without a readout', () => {
        expect(mount(PushToSolving, {props: {stage: stage(2, 3)}}).find('.stand-in').exists()).toBe(false)
    })
})
