import {computed, isRef, ref} from 'vue'
import type {Component} from 'vue'
import {vi} from 'vitest'
import {mount} from '@vue/test-utils'
import {capabilities, settings} from './fixtures'
import {createEventStreamStub} from './eventStream'
import {contextProvides} from '../composables/appContext'
import type {AppContext} from '../composables/appContext'

/** A ref key's value as a test gives it: the ref, or the plain value to wrap in one. */
type Override<T> = T | (T extends {readonly value: infer V} ? V : never)

export type ProvideOverrides = {
    [N in Exclude<keyof AppContext, 'eventStream'>]?: Override<AppContext[N]>
} & {eventStream?: Parameters<typeof createEventStreamStub>[0]}

/** The context by name; `applySettings` only when a test provides one, else components post directly. */
export type TestProvides = Omit<AppContext, 'applySettings'> & Partial<Pick<AppContext, 'applySettings'>>

/**
 * Everything `provideAppContext` provides, as test doubles, by name. `overrides` replaces a
 * key whole, except `eventStream`, whose fields go to `createEventStreamStub`. A plain value
 * for a ref key is wrapped in a ref. Mount with `contextProvides(…)` of it, which puts each
 * under the key components inject (`mountWithDefaults` does).
 */
export function createProvides(overrides: ProvideOverrides = {}): TestProvides {
    const {eventStream, ...rest} = overrides
    const provides: TestProvides = {
        settings: ref(settings()),
        cameras: ref([]),
        selectedCamera: ref(null),
        simulatorEnabled: ref(false),
        refreshSettings: vi.fn(async () => null),
        refreshCameras: vi.fn(async () => []),
        capabilities: ref(capabilities()),
        cameraStatus: ref({}),
        cameraPhase: ref({}),
        warmupEndsAt: ref({}),
        mainCamera: computed(() => null),
        guideCamera: computed(() => null),
        hasGuideCamera: computed(() => false),
        selectedCameraRole: computed(() => 'main' as const),
        eventStream: createEventStreamStub(eventStream),
    }
    for (const [key, value] of Object.entries(rest)) {
        Object.assign(provides, {[key]: typeof value === 'function' || isRef(value) ? value : ref(value)})
    }
    return provides
}

/** Mount with `createProvides(provide)`; `BaseProLock` is stubbed unless named otherwise. */
export function mountWithDefaults(
    component: Component,
    {provide = {}, props = {}, stubs = {}}: {
        provide?: ProvideOverrides
        props?: Record<string, unknown>
        stubs?: Record<string, unknown>
    } = {},
) {
    return mount(component, {
        props,
        global: {provide: contextProvides(createProvides(provide)), stubs: {BaseProLock: true, ...stubs}},
    })
}
