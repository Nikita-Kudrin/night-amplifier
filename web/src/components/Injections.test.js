import {describe, it, expect, vi} from 'vitest'
import {mount} from '@vue/test-utils'
import {ref} from 'vue'
import SettingsPanel from './SettingsPanel.vue'
import CaptureControls from './CaptureControls.vue'
import PushToPanel from './PushToPanel.vue'

// Mock dependencies that aren't injections
vi.mock('../composables/api.js', () => ({
    updateSettings: vi.fn(),
    getStackingTypes: vi.fn().mockResolvedValue([]),
    getAstapStatus: vi.fn().mockResolvedValue({ready: false}),
    getSimulatorConfig: vi.fn().mockResolvedValue({}),
}))

describe('Injection Resilience', () => {
    const commonStubs = {
        BasePanel: true,
        BaseToggle: true,
        BaseSlider: true,
        ButtonGroup: true,
        BaseAlert: true,
        BaseInfoIcon: true,
        BaseProLock: true,
    }

    it('SettingsPanel renders without crashing when capabilities injection is missing', () => {
        // We provide only mandatory non-capability injections if any, 
        // but here we want to see if it survives missing 'capabilities'
        expect(() => {
            mount(SettingsPanel, {
                global: {
                    provide: {
                        settings: ref({}),
                        refreshSettings: vi.fn(),
                        simulatorEnabled: ref(false),
                        // 'capabilities' is INTENTIONALLY MISSING
                    },
                    stubs: commonStubs,
                }
            })
        }).not.toThrow()
    })

    it('CaptureControls renders without crashing when capabilities injection is missing', () => {
        expect(() => {
            mount(CaptureControls, {
                global: {
                    provide: {
                        settings: ref({}),
                        selectedCamera: ref(null),
                        eventStream: {captureState: ref('Idle')},
                        refreshSettings: vi.fn(),
                        // 'capabilities' is INTENTIONALLY MISSING
                    },
                    stubs: commonStubs,
                }
            })
        }).not.toThrow()
    })

    it('PushToPanel renders without crashing when capabilities injection is missing', () => {
        expect(() => {
            mount(PushToPanel, {
                global: {
                    provide: {
                        eventStream: {lastEvent: ref(null)},
                        // 'capabilities' is INTENTIONALLY MISSING
                    },
                    stubs: commonStubs,
                }
            })
        }).not.toThrow()
    })
})
