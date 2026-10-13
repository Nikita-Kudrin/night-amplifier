import {ref} from 'vue'
import {vi} from 'vitest'
import {mount} from '@vue/test-utils'
import type {VueWrapper} from '@vue/test-utils'
import SettingsPanel from '../components/SettingsPanel.vue'
import BaseSlider from '../components/ui/BaseSlider.vue'
import {capabilities, settings} from './fixtures.js'
import {contextProvides} from '../composables/appContext.js'
import type {Capabilities, DeepSkyCapabilities, Settings} from '../composables/api.types.js'

/**
 * A toggle's checkbox, found by the label text next to it.
 *
 * `findAll('.toggle')[n]` was how these were selected; every control added above
 * one silently retargeted the assertion at a different setting.
 */
export function findToggleByLabel(wrapper: VueWrapper, label: string) {
    const found = wrapper
        .findAll('.toggle-label')
        .find((el) => el.text().includes(label))
    if (!found) throw new Error(`no toggle labelled "${label}"`)
    return found.find<HTMLInputElement>('input[type="checkbox"]')
}

/** A slider, found by its label. */
export function findSliderByLabel(wrapper: VueWrapper, label: string) {
    const found = wrapper.findAllComponents(BaseSlider).find((slider) => slider.props('label') === label)
    if (!found) throw new Error(`no slider labelled "${label}"`)
    return found
}

/** The server's defaults, which the panel's tests start from; each test adds what it is about. */
export function createDefaultSettings(): Settings {
    return settings()
}

/**
 * Settings as the panel's tests give them: each group with only the fields a test is
 * about, the rest left out, which the panel's mirror (`settingsMirror`) fills from the defaults.
 */
export type SettingsOverrides = {[K in keyof Settings]?: Partial<Settings[K]>}

interface PanelOverrides {
    /** `null` mounts the panel as it exists before the first payload arrives. */
    settings?: SettingsOverrides | null
    simulatorEnabled?: boolean
    capabilities?: Partial<Omit<Capabilities, 'deep_sky'>> & {deep_sky?: Partial<DeepSkyCapabilities>}
}

/** `overrides.capabilities.deep_sky` merges over a Community server's answer. */
export function createMockProvides(overrides: PanelOverrides = {}) {
    const base = capabilities()
    return {
        // `settings: null` mounts the panel as it exists before the first payload
        // arrives, which is a state the panel really renders in.
        settings: ref(
            overrides.settings === null
                ? null
                // A group a test gives replaces the default one whole, its missing fields
                // and all: what the mirror is to fill, and the wire's types cannot state.
                : {...createDefaultSettings(), ...overrides.settings} as Settings
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

export function mountSettingsPanel(provides: PanelOverrides = {}) {
    return mount(SettingsPanel, {
        global: {
            provide: contextProvides(createMockProvides(provides)),
            stubs: {
                BaseProLock: true,
            },
        },
    })
}
