import {ref} from 'vue'
import {vi} from 'vitest'
import {mount} from '@vue/test-utils'
import SettingsPanel from '../components/SettingsPanel.vue'
import {capabilities} from './fixtures.js'

/**
 * A toggle's checkbox, found by the label text next to it.
 *
 * `findAll('.toggle')[n]` was how these were selected; every control added above
 * one silently retargeted the assertion at a different setting.
 */
export function findToggleByLabel(wrapper, label) {
    const found = wrapper
        .findAll('.toggle-label')
        .find((el) => el.text().includes(label))
    if (!found) throw new Error(`no toggle labelled "${label}"`)
    return found.find('input[type="checkbox"]')
}

/** The few settings the panel's tests start from; each test adds what it is about. */
export function createDefaultSettings() {
    return {
        offset: 10,
        bin: 1,
        auto_stretch: true,
        auto_stretch_intensity: 0.3,
        stacking: true,
        background_subtraction: true,
        raw_frame_saving: {live_view: false, wanderer: false, stacking: false},
        save_stacked_image: false,
    }
}

/** `overrides.capabilities.deep_sky` merges over a Community server's answer. */
export function createMockProvides(overrides = {}) {
    const base = capabilities()
    return {
        // `settings: null` mounts the panel as it exists before the first payload
        // arrives, which is a state the panel really renders in.
        settings: ref(
            overrides.settings === null
                ? null
                : {...createDefaultSettings(), ...overrides.settings}
        ),
        refreshSettings: vi.fn().mockResolvedValue(undefined),
        simulatorEnabled: ref(overrides.simulatorEnabled ?? false),
        capabilities: ref({
            ...base,
            ...overrides.capabilities,
            deep_sky: {...base.deep_sky, ...overrides.capabilities?.deep_sky},
        }),
    }
}

export function mountSettingsPanel(provides = {}) {
    return mount(SettingsPanel, {
        global: {
            provide: createMockProvides(provides),
            stubs: {
                BaseProLock: true,
            },
        },
    })
}
