import {mount} from '@vue/test-utils'
import {nextTick} from 'vue'
import ProcessingSettings from './ProcessingSettings.vue'
import {settingsMirror} from '../utils/settings.js'
import {findToggleByLabel} from '../test/settingsPanel.js'
import {HELP_TEXTS} from '../constants'

function mountSection(props = {}, server = {}) {
    return mount(ProcessingSettings, {
        props: {settings: settingsMirror(server), rbfBackgroundAvailable: true, saturationBoostAvailable: true, ...props},
        global: {stubs: {BaseProLock: true}},
    })
}

function slider(wrapper, label) {
    return wrapper.findAllComponents({name: 'BaseSlider'}).find((s) => s.props('label') === label)
}

async function release(wrapper, label, value) {
    slider(wrapper, label).vm.$emit('update:modelValue', value)
    slider(wrapper, label).vm.$emit('change', value)
    await nextTick()
}

describe('ProcessingSettings', () => {
    afterEach(() => vi.useRealTimers())

    // It used to wait 300 ms; every settings slider now sends on release.
    it('sends the saturation strength on release, at once', async () => {
        vi.useFakeTimers()
        const wrapper = mountSection({}, {saturation_boost: true})

        await release(wrapper, 'Saturation Strength', 0.7)

        expect(wrapper.emitted('apply')).toEqual([['saturation_boost_strength', 0.7]])
    })

    it('sends a tone field as the whole eyepiece group', async () => {
        const wrapper = mountSection({}, {eyepiece: {binoview: false, intensity: 0.3}})

        await release(wrapper, 'Black level', 0.4)

        const [[key, eyepiece]] = wrapper.emitted('apply')
        expect(key).toBe('eyepiece')
        expect(eyepiece).toMatchObject({binoview: false, intensity: 0.4})
    })

    // The Eyepiece section edits the same group: a stale copy here would undo its edit.
    it('follows an eyepiece edit made elsewhere', async () => {
        const wrapper = mountSection({}, {eyepiece: {binoview: true}})

        await wrapper.setProps({settings: settingsMirror({eyepiece: {binoview: false}})})
        await release(wrapper, 'Black floor', -0.02)

        expect(wrapper.emitted('apply')[0][1]).toMatchObject({binoview: false, black_floor: -0.02})
    })

    it('keeps its edits off the panel\'s copy until it emits them', async () => {
        const settings = settingsMirror({})
        const wrapper = mountSection({settings})

        slider(wrapper, 'Black level').vm.$emit('update:modelValue', 0.9)
        await nextTick()

        expect(settings.eyepiece.intensity).not.toBe(0.9)
    })

    it('locks the RBF algorithm without the capability', () => {
        const wrapper = mountSection({rbfBackgroundAvailable: false}, {background_subtraction: true})
        const rbf = wrapper.findAll('#bg-algorithm-select option').find((o) => o.element.disabled)

        expect(rbf.text()).toContain('🔒')
    })

    it('holds dithering, background subtraction and the boost while Focus/Finder mode is on', () => {
        const wrapper = mountSection({focusMode: true})

        expect(wrapper.text()).toContain(HELP_TEXTS.focus_mode_held)
        for (const label of ['Dithering', 'Background Subtraction', 'Shadow Saturation Boost']) {
            expect(findToggleByLabel(wrapper, label).element.disabled, label).toBe(true)
        }
    })
})
