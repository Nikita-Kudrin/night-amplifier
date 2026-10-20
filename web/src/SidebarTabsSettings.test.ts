/**
 * The sidebar tabs against the real panels: what `settings.json` loaded must show in a tab
 * first opened later, switching tabs must not write, an edit must reach the server even
 * when its panel goes away inside the debounce window, and a hidden Push-To tab must
 * follow the server's telescope profiles.
 */
import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {mount, flushPromises} from '@vue/test-utils'
import type {VueWrapper} from '@vue/test-utils'
import {ref, computed} from 'vue'
import {DEFAULT_SETTINGS} from './constants'
import {findSliderByLabel} from './test/settingsPanel'
import type * as Api from './composables/api'
import type {Settings, TelescopeSettings, TelescopeSettingsInput, UpdateSettingsRequest} from './composables/api.types'

vi.mock('./composables/api', async (importOriginal) =>
    (await import('./test/mockApi')).mockApi(await importOriginal<typeof Api>(), {
        getAstapStatus: vi.fn().mockResolvedValue({ready: true, installed_databases: []}),
    }))

vi.mock('./composables/useEventStream', async (importOriginal) => {
    const {createEventStreamStub} = await import('./test/eventStream')
    return {...await importOriginal(), useEventStream: vi.fn(() => createEventStreamStub())}
})

const settingsRef = ref<Settings | null>(null)
const selectedRef = ref<string | null>('cam-a')
const errorRef = ref<string | null>(null)

// Connected, so Push-To resolves this camera's stored telescope profile on mount.
const CAMERA_INFO = {name: 'Camera A', pixel_size_x_um: 3.76, pixel_size_y_um: 3.76, max_width: 3008, max_height: 3008}
const CAMERA_A = {id: 'cam-a', name: 'Camera A', provider: 'ZWO', connected: true, role: 'main', info: CAMERA_INFO}
const camerasRef = ref([CAMERA_A])

vi.mock('./composables/useAppState', () => ({
    useAppState: vi.fn(() => ({
        loading: ref(false),
        globalError: errorRef,
        simulatorEnabled: ref(false),
        settings: settingsRef,
        refreshSettings: vi.fn().mockResolvedValue(undefined),
        // Posts only: the answer `mockApi` gives is a fixture, not these settings.
        applySettings: vi.fn(async (patch: UpdateSettingsRequest) => (await import('./composables/api')).updateSettings(patch)),
        refreshCameras: vi.fn(),
        initializeState: vi.fn(),
        connectEvents: vi.fn(),
        cameras: camerasRef,
        selection: selectedRef,
        cameraStatus: ref({}),
        cameraPhase: ref({}),
        warmupEndsAt: ref({}),
        capabilities: ref({
            has_pro: true,
            deep_sky: {advanced_rejection: true, rbf_background: true, denoise: false, ai_denoise: false},
            planetary: {advanced_stacking: true},
            push_to: {astap_solver: false},
        }),
        mainCamera: computed(() => null),
        guideCamera: computed(() => null),
        hasGuideCamera: computed(() => false),
        selectedCameraRole: computed(() => 'main'),
    })),
}))

import App from './App.vue'
import {updateSettings} from './composables/api'

const TELESCOPE: TelescopeSettings = {
    focal_length_mm: 650,
    pixel_size_x_um: 3.76,
    pixel_size_y_um: 3.76,
    sensor_width_px: 3008,
    sensor_height_px: 3008,
    barlow_coeff: 1,
}

/** As `settings.json` would load it: every value off its default. */
function loadedSettings(): Settings {
    return {
        ...DEFAULT_SETTINGS,
        eula_accepted: true,
        bin: 2,
        stacking: true,
        stacking_type: 'deep_sky',
        rejection_method: 'SigmaClip',
        rejection_sigma: 2.2,
        auto_crop: true,
        raw_frame_saving: {...DEFAULT_SETTINGS.raw_frame_saving, live_view: false, wanderer: false, stacking: true},
        eyepiece: {...DEFAULT_SETTINGS.eyepiece, binoview: true, screen_width: 133, screen_height: 71},
        telescope: TELESCOPE,
        camera_telescope_profiles: {'Camera A': {...TELESCOPE}},
        last_camera_name: 'Camera A',
    }
}

describe('Sidebar tabs and the settings round trip', () => {
    let wrapper: VueWrapper

    beforeEach(async () => {
        vi.useFakeTimers()
        settingsRef.value = loadedSettings()
        camerasRef.value = [CAMERA_A]
        selectedRef.value = 'cam-a'
        errorRef.value = null
        wrapper = mount(App, {
            attachTo: document.body,
            global: {stubs: {LiveView: true, StatusBar: true, HardwareBenchmarkOverlay: true}},
        })
        await flushPromises()
        vi.mocked(updateSettings).mockClear()
    })

    afterEach(() => {
        wrapper.unmount()
        vi.useRealTimers()
    })

    async function openTab(name: string) {
        await wrapper.find(`[data-test="tab-${name}"]`).trigger('click')
        await flushPromises()
    }

    const settingsPane = () => wrapper.find('[data-test="sidebar-settings"]')
    const sigmaSlider = () => findSliderByLabel(wrapper, 'Rejection Sigma')
    const screenWidth = () => settingsPane().find<HTMLInputElement>('input.screen-number')

    function toggleByLabel(label: string) {
        const found = settingsPane().findAll('.toggle-label').find((el) => el.text().includes(label))
        if (!found) throw new Error(`no toggle labelled "${label}"`)
        return found.find<HTMLInputElement>('input[type="checkbox"]')
    }

    it('shows the loaded values in a Settings tab opened after the load', async () => {
        await openTab('settings')

        expect(sigmaSlider().props('modelValue')).toBe(2.2)
        expect(toggleByLabel('Auto Crop').element.checked).toBe(true)
        expect(Number(screenWidth().element.value)).toBe(133)
        const bin2 = settingsPane().findAll('.btn-option').find((b) => b.text() === '2x2')
        expect(bin2?.classes()).toContain('active')
    })

    // A write here would rewrite part of the observer's config from whatever the
    // panel's mirror held, every time a tab is opened.
    it('writes nothing while tabs are opened and switched', async () => {
        expect(camerasRef.value[0].connected).toBe(true)
        for (const tab of ['settings', 'push-to', 'settings', 'settings', 'push-to', 'push-to']) {
            await openTab(tab)
        }
        await vi.advanceTimersByTimeAsync(2000)

        expect(updateSettings).not.toHaveBeenCalled()
    })

    // A hidden tab is not unmounted: its debounced save still runs on time.
    it('stores a sigma dragged just before hiding the tab, and only that key', async () => {
        await openTab('settings')
        const slider = sigmaSlider()
        slider.vm.$emit('update:modelValue', 2.6)
        slider.vm.$emit('change', 2.6)

        await openTab('settings')
        await vi.advanceTimersByTimeAsync(300)

        expect(updateSettings).toHaveBeenCalledTimes(1)
        expect(updateSettings).toHaveBeenCalledWith({rejection_sigma: 2.6})
    })

    it('stores a screen size typed just before switching to Push-To (hidden, not unmounted)', async () => {
        await openTab('settings')
        await screenWidth().setValue(150)

        await openTab('push-to')
        await vi.advanceTimersByTimeAsync(300)

        expect(updateSettings).toHaveBeenCalledTimes(1)
        expect(updateSettings).toHaveBeenCalledWith({
            eyepiece: {...loadedSettings().eyepiece, screen_width: 150},
        })
    })

    // Another client, or the Focus/Finder mode, changes settings while the tab is hidden.
    it('shows settings changed while the tab was hidden, without writing them back', async () => {
        await openTab('settings')
        await openTab('settings')

        settingsRef.value = {...loadedSettings(), rejection_sigma: 4.0, auto_crop: false}
        await flushPromises()
        await openTab('settings')
        await vi.advanceTimersByTimeAsync(2000)

        expect(sigmaSlider().props('modelValue')).toBe(4.0)
        expect(toggleByLabel('Auto Crop').element.checked).toBe(false)
        expect(updateSettings).not.toHaveBeenCalled()
    })
    // The one way a visited panel unmounts: the server drops and the error screen replaces
    // the whole layout. Vue drops an emit from an unmounted panel, so the edit must leave first.
    it('stores a screen size typed just before the panel is unmounted', async () => {
        await openTab('settings')
        await screenWidth().setValue(150)

        errorRef.value = 'Server unreachable'
        await flushPromises()
        expect(wrapper.find('[data-test="sidebar-settings"]').exists()).toBe(false)

        expect(updateSettings).toHaveBeenCalledTimes(1)
        expect(updateSettings).toHaveBeenCalledWith({
            eyepiece: {...loadedSettings().eyepiece, screen_width: 150},
        })
    })

    describe('a hidden Push-To tab', () => {
        const GUIDE_SCOPE: TelescopeSettings = {
            focal_length_mm: 200,
            pixel_size_x_um: 2.9,
            pixel_size_y_um: 2.9,
            sensor_width_px: 1936,
            sensor_height_px: 1096,
            barlow_coeff: 1,
        }
        const CAMERA_B = {
            id: 'cam-b', name: 'Camera B', provider: 'ZWO', connected: true, role: 'guide',
            info: {name: 'Camera B', pixel_size_x_um: 2.9, pixel_size_y_um: 2.9, max_width: 1936, max_height: 1096},
        }
        const pushToInputs = () =>
            wrapper.find('[data-test="sidebar-push-to"]').findAll<HTMLInputElement>('input[type="number"]').map((i) => i.element.value)

        async function visitPushTo() {
            await openTab('push-to')
            await openTab('push-to')
            await vi.advanceTimersByTimeAsync(1000)
            vi.mocked(updateSettings).mockClear()
        }

        // The guide camera connects after Push-To's first visit and the server seeds its
        // profile; clicking it on Main once wrote the main scope's 650 mm over it.
        it('keeps the guide profile the server seeded when the guide is selected on Main', async () => {
            await visitPushTo()

            settingsRef.value = {
                ...loadedSettings(),
                camera_telescope_profiles: {'Camera A': {...TELESCOPE}, 'Camera B': {...GUIDE_SCOPE}},
            }
            camerasRef.value = [CAMERA_A, CAMERA_B]
            await flushPromises()
            selectedRef.value = 'cam-b'
            await flushPromises()
            await vi.advanceTimersByTimeAsync(1000)

            // The server upserts the profiles a save names; any it names must be unchanged.
            const stored = vi.mocked(updateSettings).mock.calls.reduce<Partial<Record<string, TelescopeSettingsInput>>>(
                (profiles, [payload]) => ({...profiles, ...payload.camera_telescope_profiles}),
                {...settingsRef.value?.camera_telescope_profiles},
            )
            expect(stored['Camera B']).toMatchObject({focal_length_mm: 200})
            expect(stored['Camera A']).toMatchObject({focal_length_mm: 650})
            await openTab('push-to')
            expect(pushToInputs()[0]).toBe('200')
        })

        it('shows a focal length changed elsewhere when reopened, without writing', async () => {
            await visitPushTo()

            const edited = {...TELESCOPE, focal_length_mm: 1000}
            settingsRef.value = {...loadedSettings(), telescope: edited, camera_telescope_profiles: {'Camera A': edited}}
            await flushPromises()
            await openTab('push-to')
            await vi.advanceTimersByTimeAsync(1000)

            expect(pushToInputs()[0]).toBe('1000')
            expect(updateSettings).not.toHaveBeenCalled()
        })
    })
})
