import {describe, it, expect, vi, afterEach} from 'vitest'
import {flushPromises, mount} from '@vue/test-utils'
import type {VueWrapper} from '@vue/test-utils'
import {h, nextTick, reactive, ref} from 'vue'
import EyepieceSettings from './EyepieceSettings.vue'
import SettingsPanel from './SettingsPanel.vue'
import {mountWithDefaults} from '../test/mount'
import {settings} from '../test/fixtures'
import {findSliderByLabel} from '../test/settingsPanel'
import {DEFAULT_SETTINGS, SETTINGS_DEBOUNCE_MS} from '../constants'
import type * as Api from '../composables/api'
import type {EyepieceSettings as Eyepiece, Settings, UpdateSettingsRequest} from '../composables/api.types'

vi.mock('../composables/api', async (importOriginal) =>
    (await import('../test/mockApi')).mockApi(await importOriginal<typeof Api>()))

const EYEPIECE: Eyepiece = {
    ...DEFAULT_SETTINGS.eyepiece,
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

const field = (wrapper: VueWrapper, title: string) => wrapper.find<HTMLInputElement>(`input[title="${title}"]`)

describe('EyepieceSettings', () => {
    afterEach(() => vi.useRealTimers())

    it('sends both screen sizes typed in a row once, 300 ms after the last', async () => {
        vi.useFakeTimers()
        const wrapper = mount(EyepieceSettings, {props: {eyepiece: {...EYEPIECE}}})

        await field(wrapper, 'Width').setValue(130)
        await field(wrapper, 'Height').setValue(75)
        expect(wrapper.emitted('apply')).toBeUndefined()

        vi.advanceTimersByTime(SETTINGS_DEBOUNCE_MS)
        expect(wrapper.emitted('apply')).toEqual([['eyepiece', {screen_width: 130, screen_height: 75}]])
    })

    // The tone fields are Processing's; the panel lays these over its own copy of the group.
    it('hands over only the fields it changed, a pending size with them', async () => {
        vi.useFakeTimers()
        const wrapper = mount(EyepieceSettings, {props: {eyepiece: {...EYEPIECE}}})

        await field(wrapper, 'Width').setValue(130)
        await wrapper.find('#eyepiece-stream-resolution-select').setValue('native')
        vi.advanceTimersByTime(SETTINGS_DEBOUNCE_MS)

        expect(wrapper.emitted('apply')).toEqual([['eyepiece', {screen_width: 130, stream_resolution: 'native'}]])
    })

    it('keeps a typed size over a sync until it is sent', async () => {
        vi.useFakeTimers()
        const eyepiece = reactive({...EYEPIECE})
        const wrapper = mount(EyepieceSettings, {props: {eyepiece}})

        await field(wrapper, 'Width').setValue(130)
        Object.assign(eyepiece, {screen_width: 120, screen_height: 80})
        await nextTick()

        expect(field(wrapper, 'Width').element.value).toBe('130')
        expect(field(wrapper, 'Height').element.value).toBe('80')
        vi.advanceTimersByTime(SETTINGS_DEBOUNCE_MS)
        expect(wrapper.emitted('apply')).toEqual([['eyepiece', {screen_width: 130}]])
    })

    // `v-model.number` handed an emptied field over as '', and the server refused the group.
    it.each([
        ['Width', ''],
        ['Width', '-3'],
        ['Resolution X', ''],
        ['Resolution X', '1920.5'],
        ['Resolution Y', '0'],
    ])('reverts %s set to %j to its last value, sending nothing', async (title, typed) => {
        vi.useFakeTimers()
        const wrapper = mount(EyepieceSettings, {props: {eyepiece: {...EYEPIECE}}})
        const before = field(wrapper, title).element.value

        await field(wrapper, title).setValue(typed)
        vi.advanceTimersByTime(SETTINGS_DEBOUNCE_MS)

        expect(field(wrapper, title).element.value).toBe(before)
        expect(wrapper.emitted('apply')).toBeUndefined()
    })

    it('reverts an emptied size to the one waiting to be sent', async () => {
        vi.useFakeTimers()
        const wrapper = mount(EyepieceSettings, {props: {eyepiece: {...EYEPIECE}}})

        await field(wrapper, 'Width').setValue(130)
        await field(wrapper, 'Width').setValue('')
        vi.advanceTimersByTime(SETTINGS_DEBOUNCE_MS)

        expect(field(wrapper, 'Width').element.value).toBe('130')
        expect(wrapper.emitted('apply')).toEqual([['eyepiece', {screen_width: 130}]])
    })

    // Vue drops an emit from an unmounted component, so a size typed just before the
    // Settings tab closed never reached the server.
    it('sends a pending screen size when it unmounts', async () => {
        vi.useFakeTimers()
        const onApply = vi.fn()
        const wrapper = mount({render: () => h(EyepieceSettings, {eyepiece: {...EYEPIECE}, onApply})})

        await field(wrapper, 'Width').setValue(130)
        wrapper.unmount()

        expect(onApply).toHaveBeenCalledExactlyOnceWith('eyepiece', {screen_width: 130})
        vi.advanceTimersByTime(SETTINGS_DEBOUNCE_MS)
        expect(onApply).toHaveBeenCalledTimes(1)
    })
})

describe('EyepieceSettings in the panel', () => {
    afterEach(() => vi.useRealTimers())

    /** Binoview on, so the screen size fields are shown. */
    const binoview = () => settings({eyepiece: {binoview: true}})

    /** The panel under a stand-in for the settings store: every save's answer is adopted. */
    function mountPanel() {
        const server = ref(binoview())
        const applySettings = vi.fn(async (patch: UpdateSettingsRequest) => {
            server.value = {...server.value, ...JSON.parse(JSON.stringify(patch))}
            return server.value
        })
        const wrapper = mountWithDefaults(SettingsPanel, {provide: {settings: server, applySettings}})
        return {wrapper, server, applySettings}
    }

    async function releaseBlackLevel(wrapper: VueWrapper, value: number) {
        const blackLevel = findSliderByLabel(wrapper, 'Black level')
        const range = blackLevel.find<HTMLInputElement>('input[type="range"]')
        range.element.value = String(value)
        await range.trigger('input')
        await range.trigger('change')
        await nextTick()
    }

    // Both sections wrote the whole group from their own copies: Processing's (with the old
    // size) landed inside the debounce, Eyepiece's mirror took it over the typed value, and
    // the delayed send posted the old size.
    it('keeps a typed screen size when Processing commits the same group inside the delay', async () => {
        vi.useFakeTimers()
        const {wrapper, applySettings, server} = mountPanel()
        const before = server.value.eyepiece.screen_width

        await field(wrapper, 'Width').setValue(before + 10)
        await releaseBlackLevel(wrapper, 0.5)
        await vi.advanceTimersByTimeAsync(SETTINGS_DEBOUNCE_MS)
        await flushPromises()

        expect(applySettings).toHaveBeenCalledTimes(2)
        expect(applySettings.mock.calls.at(-1)?.[0].eyepiece).toMatchObject({screen_width: before + 10, intensity: 0.5})
        expect(server.value.eyepiece).toMatchObject({screen_width: before + 10, intensity: 0.5})
    })

    // An answer to the earlier write predates the later one: it must not take the later
    // write's field back off the screen, or out of the next write.
    it('keeps a field in flight over an earlier write\'s answer', async () => {
        vi.useFakeTimers()
        const answers: {patch: UpdateSettingsRequest, resolve: (answer: Settings) => void}[] = []
        const server = ref(binoview())
        const applySettings = vi.fn((patch: UpdateSettingsRequest) =>
            new Promise<Settings>((resolve) => answers.push({patch, resolve})))
        const wrapper = mountWithDefaults(SettingsPanel, {provide: {settings: server, applySettings}})

        await releaseBlackLevel(wrapper, 0.5)
        await field(wrapper, 'Width').setValue(150)
        await vi.advanceTimersByTimeAsync(SETTINGS_DEBOUNCE_MS)
        expect(answers).toHaveLength(2)

        // The panel sends the group whole.
        server.value = {...server.value, eyepiece: {...server.value.eyepiece, ...answers[0].patch.eyepiece}}
        answers[0].resolve(server.value)
        await flushPromises()
        expect(field(wrapper, 'Width').element.value).toBe('150')

        await wrapper.find('#eyepiece-stream-resolution-select').setValue('native')
        expect(applySettings.mock.calls.at(-1)?.[0].eyepiece).toMatchObject({
            screen_width: 150,
            intensity: 0.5,
            stream_resolution: 'native',
        })
    })

    it('never sends an emptied screen size', async () => {
        vi.useFakeTimers()
        const {wrapper, applySettings} = mountPanel()

        await field(wrapper, 'Width').setValue('')
        await vi.advanceTimersByTimeAsync(SETTINGS_DEBOUNCE_MS)
        await flushPromises()

        expect(applySettings).not.toHaveBeenCalled()
    })
})
