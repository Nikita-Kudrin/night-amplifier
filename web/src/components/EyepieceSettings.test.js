import {describe, it, expect, vi, afterEach} from 'vitest'
import {mount} from '@vue/test-utils'
import {nextTick, reactive} from 'vue'
import EyepieceSettings from './EyepieceSettings.vue'

const EYEPIECE = {
    binoview: true,
    circular_view: true,
    stream_resolution: 'qhd1440',
    intensity: 0.5,
    screen_width: 120,
    screen_height: 70,
    screen_measurement: 'mm',
    screen_resolution_x: 2560,
    screen_resolution_y: 1440,
}

describe('EyepieceSettings', () => {
    afterEach(() => vi.useRealTimers())

    it('sends both screen sizes typed in a row once, 300 ms after the last', async () => {
        vi.useFakeTimers()
        const wrapper = mount(EyepieceSettings, {props: {eyepiece: {...EYEPIECE}}})
        const [width, height] = wrapper.findAll('input[type="number"]')

        await width.setValue(130)
        await height.setValue(75)
        expect(wrapper.emitted('apply')).toBeUndefined()

        vi.advanceTimersByTime(300)
        expect(wrapper.emitted('apply')).toEqual([
            ['eyepiece', expect.objectContaining({screen_width: 130, screen_height: 75})],
        ])
    })

    // The Processing section edits the same group's tone fields in place; a mirror that
    // missed them would send the old values back with the next eyepiece edit.
    it('sends the tone fields the panel edited in place', async () => {
        const eyepiece = reactive({...EYEPIECE})
        const wrapper = mount(EyepieceSettings, {props: {eyepiece}})

        eyepiece.intensity = 0.8
        await nextTick()
        await wrapper.find('#eyepiece-stream-resolution-select').setValue('native')

        expect(wrapper.emitted('apply').at(-1)).toEqual([
            'eyepiece',
            expect.objectContaining({intensity: 0.8, stream_resolution: 'native'}),
        ])
    })
})
