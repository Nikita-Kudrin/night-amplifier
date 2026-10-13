import {describe, it, expect, vi} from 'vitest'
import {mount} from '@vue/test-utils'
import {ref} from 'vue'
import SettingsPanel from './SettingsPanel.vue'
import CaptureControls from './CaptureControls.vue'
import PushToPanel from './PushToPanel.vue'
import {createEventStreamStub} from '../test/eventStream.js'
import {settings} from '../test/fixtures.js'
import {contextProvides} from '../composables/appContext.js'

// Mock dependencies that aren't injections
vi.mock('../composables/api.js', async (importOriginal) =>
    (await import('../test/mockApi.js')).mockApi(await importOriginal(), {
        getStackingTypes: vi.fn(async () => []),
        getAstapStatus: vi.fn(async () => (await import('../test/fixtures.js')).astapStatus({ready: false})),
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
        // Only the non-capability injections it needs; 'capabilities' is INTENTIONALLY MISSING.
        expect(() => {
            mount(SettingsPanel, {
                global: {
                    provide: contextProvides({
                        settings: ref(settings()),
                        refreshSettings: vi.fn(async () => null),
                        simulatorEnabled: ref(false),
                    }),
                    stubs: commonStubs,
                }
            })
        }).not.toThrow()
    })

    it('CaptureControls renders without crashing when capabilities injection is missing', () => {
        expect(() => {
            mount(CaptureControls, {
                global: {
                    provide: contextProvides({
                        settings: ref(settings()),
                        selectedCamera: ref(null),
                        eventStream: createEventStreamStub(),
                        refreshSettings: vi.fn(async () => null),
                    }),
                    stubs: commonStubs,
                }
            })
        }).not.toThrow()
    })

    it('PushToPanel renders without crashing when capabilities injection is missing', () => {
        expect(() => {
            mount(PushToPanel, {
                global: {
                    provide: contextProvides({
                        settings: ref(settings()),
                        eventStream: createEventStreamStub(),
                    }),
                    stubs: commonStubs,
                }
            })
        }).not.toThrow()
    })
})
