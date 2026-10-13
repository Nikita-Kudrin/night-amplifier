import {computed, ref} from 'vue'
import {vi} from 'vitest'
import {mount} from '@vue/test-utils'
import {capabilities, settings} from './fixtures.js'
import {createEventStreamStub} from './eventStream.js'

/**
 * Everything `provideAppContext` provides, as test doubles. `overrides` replaces a key
 * whole, except `eventStream`, whose fields go to `createEventStreamStub`. A plain value
 * for a ref key is wrapped in a ref.
 */
export function createProvides(overrides = {}) {
    const {eventStream, ...rest} = overrides
    const provides = {
        settings: ref(settings()),
        cameras: ref([]),
        selectedCamera: ref(null),
        simulatorEnabled: ref(false),
        refreshSettings: vi.fn(async () => {}),
        refreshCameras: vi.fn(async () => {}),
        capabilities: ref(capabilities()),
        cameraStatus: ref({}),
        cameraPhase: ref({}),
        warmupEndsAt: ref({}),
        mainCamera: computed(() => null),
        guideCamera: computed(() => null),
        hasGuideCamera: computed(() => false),
        selectedCameraRole: computed(() => 'main'),
    }
    for (const [key, value] of Object.entries(rest)) {
        provides[key] = typeof value === 'function' || value?.__v_isRef ? value : ref(value)
    }
    provides.eventStream = createEventStreamStub(eventStream)
    return provides
}

/** Mount with `createProvides(provide)`; `BaseProLock` is stubbed unless named otherwise. */
export function mountWithDefaults(component, {provide = {}, props = {}, stubs = {}} = {}) {
    return mount(component, {
        props,
        global: {provide: createProvides(provide), stubs: {BaseProLock: true, ...stubs}},
    })
}
