import {describe, it, expect, vi, beforeEach} from 'vitest'
import {mount, flushPromises} from '@vue/test-utils'
import {ref} from 'vue'
import PushToPanel from './PushToPanel.vue'

// Mock the full API module used by PushToPanel and its composables
vi.mock('../composables/api.js', () => ({
    getAstapStatus: vi.fn(),
    getAstapDatabases: vi.fn(),
    updateSettings: vi.fn().mockResolvedValue({}),
    updatePushToConfig: vi.fn().mockResolvedValue({}),
    searchCatalog: vi.fn().mockResolvedValue([]),
    getPushToStatus: vi.fn().mockResolvedValue({
        current_target: null,
        last_position: null,
        direction: null,
        is_solving: false,
    }),
    setTargetByName: vi.fn(),
    setTargetByCoordinates: vi.fn(),
    clearTarget: vi.fn(),
    cancelPushToSolve: vi.fn(),
}))

import {getAstapStatus, getAstapDatabases, updatePushToConfig} from '../composables/api.js'

// ─── FOV calculation helpers ──────────────────────────────────────────────────
//
// The frontend computes:  fovY = (h * py_um / 1000) / fl * (180 / PI)
// With fl=10 mm and py=4.63 μm the coefficient is ≈ 0.026528°/px.
//
// These helpers produce settings that yield a specific, predictable fovY.

const FL = 10       // focal length mm
const PY = 4.63     // pixel size μm (square pixels)
const W = 100       // sensor width px (arbitrary — does not affect fovY)

/**
 * Return sensor height (px) that produces approximately the target fovY.
 * The formula is exact for the small-angle approximation used by the frontend.
 */
function heightForFov(targetFovDeg) {
    return Math.round(targetFovDeg / (PY / 1000 / FL * (180 / Math.PI)))
}

/** Build a settings ref that drives the telescope composable to the given fovY. */
function settingsForFov(sensorHeightPx) {
    return ref({
        telescope: {
            focal_length_mm: FL,
            pixel_size_x_um: PY,
            pixel_size_y_um: PY,
            sensor_width_px: W,
            sensor_height_px: sensorHeightPx,
            barlow_coeff: 1.0,
        },
    })
}

// ─── ASTAP status factories ───────────────────────────────────────────────────

const DB_CONFIGS = {
    D80: {id: 'D80', database_path: '/db/d80', min_fov_deg: 0.15, max_fov_deg: 6.0},
    G05: {id: 'G05', database_path: '/db/g05', min_fov_deg: 3.0, max_fov_deg: 20.0},
    W08: {id: 'W08', database_path: '/db/w08', min_fov_deg: 20.0, max_fov_deg: 80.0},
}

function astapStatus(activeDbId, ...extraDbs) {
    return {
        binary_installed: true,
        database_installed: true,
        database_type: activeDbId,
        installed_databases: [DB_CONFIGS[activeDbId], ...extraDbs.map(id => DB_CONFIGS[id])],
        ready: true,
    }
}

// All available databases (from getAstapDatabases API) with numeric FOV fields
const ALL_DATABASES = [
    {id: 'D80', description: 'General Purpose', min_fov_deg: 0.15, max_fov_deg: 6.0, size: '~1.3GB', installed: false},
    {id: 'G05', description: 'Camera Lenses', min_fov_deg: 3.0, max_fov_deg: 20.0, size: '~100MB', installed: false},
    {id: 'W08', description: 'Fisheye Lenses', min_fov_deg: 20.0, max_fov_deg: 80.0, size: '<1MB', installed: false},
]

/** Build a getAstapDatabases mock response with specified DBs marked as installed */
function databases(installedIds = []) {
    return ALL_DATABASES.map(db => ({...db, installed: installedIds.includes(db.id)}))
}

// ─── Component mounting ───────────────────────────────────────────────────────

function mountPanel(settings, mockedAstapStatus, mockedDatabases = databases()) {
    getAstapStatus.mockResolvedValue(mockedAstapStatus)
    getAstapDatabases.mockResolvedValue(mockedDatabases)

    return mount(PushToPanel, {
        global: {
            provide: {
                settings,
                cameras: ref([]),
                selectedCamera: ref(null),
                capabilities: ref({
                    has_pro: true,
                    push_to: {astap_solver: true},
                    deep_sky: {advanced_rejection: false, rbf_background: false},
                    planetary: {advanced_stacking: false},
                }),
                eventStream: {
                    lastEvent: ref(null),
                    currentTarget: ref(null),
                    pushDirection: ref(null),
                    isSolving: ref(false),
                    astapInstallProgress: ref(null),
                    clearAstapInstallProgress: vi.fn(),
                    clearPlateSolving: vi.fn(),
                },
            },
            stubs: {
                AstapInstallOverlay: true,
                BaseProLock: true,
            },
        },
    })
}

/** Computed fovY for a sensor height using the frontend formula */
function computedFovY(h) {
    return (h * PY / 1000) / FL * (180 / Math.PI)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

describe('PushToPanel – FOV warning', () => {
    beforeEach(() => {
        vi.clearAllMocks()
    })

    // ── No-warning conditions ────────────────────────────────────────────────

    describe('no warning shown when', () => {
        it('ASTAP status is null (plugin unavailable)', async () => {
            getAstapStatus.mockResolvedValue(null)
            const wrapper = mountPanel(settingsForFov(100), null)
            await flushPromises()

            expect(wrapper.find('.fov-warning-btn').exists()).toBe(false)
        })

        it('ASTAP status has no database_type', async () => {
            const status = {binary_installed: false, database_installed: false, installed_databases: [], ready: false}
            const wrapper = mountPanel(settingsForFov(100), status)
            await flushPromises()

            expect(wrapper.find('.fov-warning-btn').exists()).toBe(false)
        })

        it('database_type is not found in installed_databases', async () => {
            const status = {
                binary_installed: true,
                database_installed: true,
                database_type: 'D80',
                installed_databases: [], // active DB not listed
                ready: true,
            }
            const wrapper = mountPanel(settingsForFov(100), status)
            await flushPromises()

            expect(wrapper.find('.fov-warning-btn').exists()).toBe(false)
        })

        it('no telescope settings provided (calculatedFov is null)', async () => {
            const emptySettings = ref({}) // no telescope key
            const wrapper = mountPanel(emptySettings, astapStatus('D80'))
            await flushPromises()

            expect(wrapper.find('.fov-warning-btn').exists()).toBe(false)
        })

        // ── D80 in-range ─────────────────────────────────────────────────────

        it('D80: FOV exactly at minimum boundary (0.15°)', async () => {
            // h=6 → fovY ≈ 0.159° ≥ 0.15° (D80 min)
            expect(computedFovY(6)).toBeGreaterThanOrEqual(0.15)
            const wrapper = mountPanel(settingsForFov(6), astapStatus('D80'))
            await flushPromises()

            expect(wrapper.find('.fov-warning-btn').exists()).toBe(false)
        })

        it('D80: FOV well within range (≈ 2.5°)', async () => {
            const h = heightForFov(2.5)
            const wrapper = mountPanel(settingsForFov(h), astapStatus('D80'))
            await flushPromises()

            expect(wrapper.find('.fov-warning-btn').exists()).toBe(false)
        })

        it('D80: FOV exactly at maximum boundary (6°)', async () => {
            // h=226 → fovY ≈ 5.995° ≤ 6.0° (D80 max)
            expect(computedFovY(226)).toBeLessThanOrEqual(6.0)
            const wrapper = mountPanel(settingsForFov(226), astapStatus('D80'))
            await flushPromises()

            expect(wrapper.find('.fov-warning-btn').exists()).toBe(false)
        })

        // ── G05 in-range ─────────────────────────────────────────────────────

        it('G05: FOV exactly at minimum boundary (3°)', async () => {
            // h=114 → fovY ≈ 3.024° ≥ 3.0° (G05 min)
            expect(computedFovY(114)).toBeGreaterThanOrEqual(3.0)
            const wrapper = mountPanel(settingsForFov(114), astapStatus('G05'))
            await flushPromises()

            expect(wrapper.find('.fov-warning-btn').exists()).toBe(false)
        })

        it('G05: FOV well within range (≈ 10°)', async () => {
            const h = heightForFov(10)
            const wrapper = mountPanel(settingsForFov(h), astapStatus('G05'))
            await flushPromises()

            expect(wrapper.find('.fov-warning-btn').exists()).toBe(false)
        })

        it('G05: FOV at maximum boundary — G05 max = W08 min (20°)', async () => {
            // h=753 → fovY ≈ 19.975° ≤ 20.0° (G05 max)
            expect(computedFovY(753)).toBeLessThanOrEqual(20.0)
            const wrapper = mountPanel(settingsForFov(753), astapStatus('G05'))
            await flushPromises()

            expect(wrapper.find('.fov-warning-btn').exists()).toBe(false)
        })

        // ── W08 in-range ─────────────────────────────────────────────────────

        it('W08: FOV exactly at minimum boundary (20°)', async () => {
            // h=755 → fovY ≈ 20.028° ≥ 20.0° (W08 min)
            expect(computedFovY(755)).toBeGreaterThanOrEqual(20.0)
            const wrapper = mountPanel(settingsForFov(755), astapStatus('W08'))
            await flushPromises()

            expect(wrapper.find('.fov-warning-btn').exists()).toBe(false)
        })

        it('W08: FOV well within range (≈ 50°)', async () => {
            const h = heightForFov(50)
            const wrapper = mountPanel(settingsForFov(h), astapStatus('W08'))
            await flushPromises()

            expect(wrapper.find('.fov-warning-btn').exists()).toBe(false)
        })

        it('W08: FOV exactly at maximum boundary (80°)', async () => {
            // h=3015 → fovY ≈ 79.98° ≤ 80.0° (W08 max)
            expect(computedFovY(3015)).toBeLessThanOrEqual(80.0)
            const wrapper = mountPanel(settingsForFov(3015), astapStatus('W08'))
            await flushPromises()

            expect(wrapper.find('.fov-warning-btn').exists()).toBe(false)
        })
    })

    // ── Warning conditions ───────────────────────────────────────────────────

    describe('warning shown when', () => {
        // ── D80 out-of-range ─────────────────────────────────────────────────

        it('D80: FOV too narrow (0.10° < 0.15° min)', async () => {
            // h=4 → fovY ≈ 0.106° < D80 min (0.15°)
            expect(computedFovY(4)).toBeLessThan(0.15)
            const wrapper = mountPanel(settingsForFov(4), astapStatus('D80'))
            await flushPromises()

            expect(wrapper.find('.fov-warning-btn').exists()).toBe(true)
        })

        it('D80: FOV too narrow — message says "too narrow"', async () => {
            const wrapper = mountPanel(settingsForFov(4), astapStatus('D80'))
            await flushPromises()
            await wrapper.find('.fov-warning-btn').trigger('click')

            expect(wrapper.find('.alert-warning').text()).toContain('too narrow')
        })

        it('D80: FOV too wide (> 6° max)', async () => {
            // h=227 → fovY ≈ 6.022° > D80 max (6°)
            expect(computedFovY(227)).toBeGreaterThan(6.0)
            const wrapper = mountPanel(settingsForFov(227), astapStatus('D80'))
            await flushPromises()

            expect(wrapper.find('.fov-warning-btn').exists()).toBe(true)
        })

        it('D80: FOV too wide — message says "too wide"', async () => {
            const wrapper = mountPanel(settingsForFov(227), astapStatus('D80'))
            await flushPromises()
            await wrapper.find('.fov-warning-btn').trigger('click')

            expect(wrapper.find('.alert-warning').text()).toContain('too wide')
        })

        it('D80: warning message mentions the database name', async () => {
            const wrapper = mountPanel(settingsForFov(4), astapStatus('D80'))
            await flushPromises()
            await wrapper.find('.fov-warning-btn').trigger('click')

            expect(wrapper.find('.alert-warning').text()).toContain('D80')
        })

        // ── G05 out-of-range ─────────────────────────────────────────────────

        it('G05: FOV too narrow (< 3° min)', async () => {
            // h=113 → fovY ≈ 2.997° < G05 min (3°)
            expect(computedFovY(113)).toBeLessThan(3.0)
            const wrapper = mountPanel(settingsForFov(113), astapStatus('G05'))
            await flushPromises()

            expect(wrapper.find('.fov-warning-btn').exists()).toBe(true)
        })

        it('G05: FOV too narrow — message says "too narrow"', async () => {
            const wrapper = mountPanel(settingsForFov(113), astapStatus('G05'))
            await flushPromises()
            await wrapper.find('.fov-warning-btn').trigger('click')

            expect(wrapper.find('.alert-warning').text()).toContain('too narrow')
        })

        it('G05: FOV too wide (> 20° max)', async () => {
            // h=754 → fovY ≈ 20.002° > G05 max (20°)
            expect(computedFovY(754)).toBeGreaterThan(20.0)
            const wrapper = mountPanel(settingsForFov(754), astapStatus('G05'))
            await flushPromises()

            expect(wrapper.find('.fov-warning-btn').exists()).toBe(true)
        })

        it('G05: FOV too wide — message says "too wide"', async () => {
            const wrapper = mountPanel(settingsForFov(754), astapStatus('G05'))
            await flushPromises()
            await wrapper.find('.fov-warning-btn').trigger('click')

            expect(wrapper.find('.alert-warning').text()).toContain('too wide')
        })

        it('G05: warning message mentions the database name', async () => {
            const wrapper = mountPanel(settingsForFov(113), astapStatus('G05'))
            await flushPromises()
            await wrapper.find('.fov-warning-btn').trigger('click')

            expect(wrapper.find('.alert-warning').text()).toContain('G05')
        })

        // ── W08 out-of-range ─────────────────────────────────────────────────

        it('W08: FOV too narrow (< 20° min)', async () => {
            // h=753 → fovY ≈ 19.975° < W08 min (20°)
            expect(computedFovY(753)).toBeLessThan(20.0)
            const wrapper = mountPanel(settingsForFov(753), astapStatus('W08'))
            await flushPromises()

            expect(wrapper.find('.fov-warning-btn').exists()).toBe(true)
        })

        it('W08: FOV too narrow — message says "too narrow"', async () => {
            const wrapper = mountPanel(settingsForFov(753), astapStatus('W08'))
            await flushPromises()
            await wrapper.find('.fov-warning-btn').trigger('click')

            expect(wrapper.find('.alert-warning').text()).toContain('too narrow')
        })

        it('W08: FOV too wide (> 80° max)', async () => {
            // h=3017 → fovY ≈ 80.013° > W08 max (80°)
            expect(computedFovY(3017)).toBeGreaterThan(80.0)
            const wrapper = mountPanel(settingsForFov(3017), astapStatus('W08'))
            await flushPromises()

            expect(wrapper.find('.fov-warning-btn').exists()).toBe(true)
        })

        it('W08: FOV too wide — message says "too wide"', async () => {
            const wrapper = mountPanel(settingsForFov(3017), astapStatus('W08'))
            await flushPromises()
            await wrapper.find('.fov-warning-btn').trigger('click')

            expect(wrapper.find('.alert-warning').text()).toContain('too wide')
        })

        it('W08: warning message mentions the database name', async () => {
            const wrapper = mountPanel(settingsForFov(753), astapStatus('W08'))
            await flushPromises()
            await wrapper.find('.fov-warning-btn').trigger('click')

            expect(wrapper.find('.alert-warning').text()).toContain('W08')
        })
    })

    // ── Boundary edge cases ──────────────────────────────────────────────────

    describe('boundary edge cases', () => {
        it('D80/G05 overlap: FOV 6° fits D80 but also G05 — correct active DB determines warning', async () => {
            // fovY ≈ 5.995° (within both D80 and G05)
            const h = 226 // fovY ≈ 5.995°

            // Active DB = D80 → no warning (5.995° ≤ D80 max 6°)
            const wrapperD80 = mountPanel(settingsForFov(h), astapStatus('D80'))
            await flushPromises()
            expect(wrapperD80.find('.fov-warning-btn').exists()).toBe(false)

            // Active DB = G05 → no warning (5.995° ≥ G05 min 3°)
            const wrapperG05 = mountPanel(settingsForFov(h), astapStatus('G05'))
            await flushPromises()
            expect(wrapperG05.find('.fov-warning-btn').exists()).toBe(false)
        })

        it('D80/G05 overlap: FOV 6.02° is too wide for D80 but fine for G05', async () => {
            const h = 227 // fovY ≈ 6.022°

            // Active DB = D80 → warning (6.022° > D80 max 6°)
            const wrapperD80 = mountPanel(settingsForFov(h), astapStatus('D80'))
            await flushPromises()
            expect(wrapperD80.find('.fov-warning-btn').exists()).toBe(true)

            // Active DB = G05 → no warning (6.022° is within G05 3°–20°)
            const wrapperG05 = mountPanel(settingsForFov(h), astapStatus('G05'))
            await flushPromises()
            expect(wrapperG05.find('.fov-warning-btn').exists()).toBe(false)
        })

        it('G05/W08 boundary: FOV 19.97° is fine for G05 but too narrow for W08', async () => {
            const h = 753 // fovY ≈ 19.975° (below 20°)

            // Active DB = G05 → no warning (19.975° ≤ G05 max 20°)
            const wrapperG05 = mountPanel(settingsForFov(h), astapStatus('G05'))
            await flushPromises()
            expect(wrapperG05.find('.fov-warning-btn').exists()).toBe(false)

            // Active DB = W08 → warning (19.975° < W08 min 20°)
            const wrapperW08 = mountPanel(settingsForFov(h), astapStatus('W08'))
            await flushPromises()
            expect(wrapperW08.find('.fov-warning-btn').exists()).toBe(true)
        })

        it('G05/W08 boundary: FOV 20.002° is too wide for G05 but fine for W08', async () => {
            const h = 754 // fovY ≈ 20.002° (above 20°)

            // Active DB = G05 → warning (20.002° > G05 max 20°)
            const wrapperG05 = mountPanel(settingsForFov(h), astapStatus('G05'))
            await flushPromises()
            expect(wrapperG05.find('.fov-warning-btn').exists()).toBe(true)

            // Active DB = W08 → no warning (20.002° ≥ W08 min 20°)
            const wrapperW08 = mountPanel(settingsForFov(h), astapStatus('W08'))
            await flushPromises()
            expect(wrapperW08.find('.fov-warning-btn').exists()).toBe(false)
        })

        it('FOV below all databases is a warning regardless of which DB is active', async () => {
            // h=2 → fovY ≈ 0.053° — below D80 min (0.15°)
            expect(computedFovY(2)).toBeLessThan(0.15)
            const wrapper = mountPanel(settingsForFov(2), astapStatus('D80'))
            await flushPromises()

            expect(wrapper.find('.fov-warning-btn').exists()).toBe(true)
        })

        it('FOV above all databases is a warning for W08', async () => {
            // h=3020 → fovY ≈ 80.1° — above W08 max (80°)
            expect(computedFovY(3020)).toBeGreaterThan(80.0)
            const wrapper = mountPanel(settingsForFov(3020), astapStatus('W08'))
            await flushPromises()

            expect(wrapper.find('.fov-warning-btn').exists()).toBe(true)
        })
    })

    // ── UI interaction ───────────────────────────────────────────────────────

    describe('UI interaction', () => {
        it('warning notification is hidden by default even when warning exists', async () => {
            const wrapper = mountPanel(settingsForFov(4), astapStatus('D80'))
            await flushPromises()

            // Icon visible, but alert not shown until clicked
            expect(wrapper.find('.fov-warning-btn').exists()).toBe(true)
            expect(wrapper.find('.alert-warning').exists()).toBe(false)
        })

        it('clicking warning icon shows the notification', async () => {
            const wrapper = mountPanel(settingsForFov(4), astapStatus('D80'))
            await flushPromises()

            await wrapper.find('.fov-warning-btn').trigger('click')

            expect(wrapper.find('.alert-warning').exists()).toBe(true)
        })

        it('clicking warning icon again hides the notification', async () => {
            const wrapper = mountPanel(settingsForFov(4), astapStatus('D80'))
            await flushPromises()

            const btn = wrapper.find('.fov-warning-btn')
            await btn.trigger('click')
            expect(wrapper.find('.alert-warning').exists()).toBe(true)

            await btn.trigger('click')
            expect(wrapper.find('.alert-warning').exists()).toBe(false)
        })

        it('dismissing the notification hides it', async () => {
            const wrapper = mountPanel(settingsForFov(4), astapStatus('D80'))
            await flushPromises()

            await wrapper.find('.fov-warning-btn').trigger('click')
            expect(wrapper.find('.alert-warning').exists()).toBe(true)

            // The BaseAlert emits 'dismiss' when its close button is clicked
            await wrapper.find('.alert-warning .btn-close').trigger('click')
            expect(wrapper.find('.alert-warning').exists()).toBe(false)
        })

        it('warning icon has a descriptive title attribute', async () => {
            const wrapper = mountPanel(settingsForFov(4), astapStatus('D80'))
            await flushPromises()

            const btn = wrapper.find('.fov-warning-btn')
            expect(btn.attributes('title')).toBeTruthy()
        })
    })

    // ── Database suggestions ─────────────────────────────────────────────────

    describe('FOV warning suggestions', () => {
        it('suggests switching to an installed database that covers the FOV', async () => {
            // Active DB = D80, FOV ≈ 6.02° (too wide for D80 max=6°), G05 installed covers 3-20°
            const wrapper = mountPanel(
                settingsForFov(227),
                astapStatus('D80', 'G05'),
                databases(['D80', 'G05']),
            )
            await flushPromises()
            await wrapper.find('.fov-warning-btn').trigger('click')

            const text = wrapper.find('.alert-warning').text()
            expect(text).toContain('too wide')
            expect(text).toContain('Switch to G05')
        })

        it('suggests downloading a database that is not installed', async () => {
            // Active DB = D80, FOV ≈ 6.02° (too wide), G05 NOT installed
            const wrapper = mountPanel(
                settingsForFov(227),
                astapStatus('D80'),
                databases(['D80']),
            )
            await flushPromises()
            await wrapper.find('.fov-warning-btn').trigger('click')

            const text = wrapper.find('.alert-warning').text()
            expect(text).toContain('too wide')
            expect(text).toContain('download G05')
        })

        it('suggests both switching and downloading when multiple databases match', async () => {
            // Active DB = W08, FOV ≈ 5° (too narrow for W08 min=20°)
            // D80 (0.15-6°) covers it — installed
            // G05 (3-20°) covers it — NOT installed
            const h = heightForFov(5)
            const wrapper = mountPanel(
                settingsForFov(h),
                astapStatus('W08', 'D80'),
                databases(['W08', 'D80']),
            )
            await flushPromises()
            await wrapper.find('.fov-warning-btn').trigger('click')

            const text = wrapper.find('.alert-warning').text()
            expect(text).toContain('too narrow')
            expect(text).toContain('Switch to D80')
            expect(text).toContain('download G05')
        })

        it('shows no suggestion when FOV is below all databases', async () => {
            // FOV ≈ 0.053° — below D80 min (0.15°)
            const wrapper = mountPanel(
                settingsForFov(2),
                astapStatus('D80'),
                databases(['D80']),
            )
            await flushPromises()
            await wrapper.find('.fov-warning-btn').trigger('click')

            const text = wrapper.find('.alert-warning').text()
            expect(text).toContain('too narrow')
            expect(text).not.toContain('Switch')
            expect(text).not.toContain('download')
        })

        it('shows no suggestion when FOV is above all databases', async () => {
            // fovY ≈ 80.1° — above W08 max (80°)
            const wrapper = mountPanel(
                settingsForFov(3020),
                astapStatus('W08'),
                databases(['D80', 'G05', 'W08']),
            )
            await flushPromises()
            await wrapper.find('.fov-warning-btn').trigger('click')

            const text = wrapper.find('.alert-warning').text()
            expect(text).toContain('too wide')
            expect(text).not.toContain('Switch')
            expect(text).not.toContain('download')
        })

        it('does not suggest the active database itself', async () => {
            // Active = G05, FOV ≈ 1° (too narrow for G05 min=3°), D80 covers 0.15-6°
            const h = heightForFov(1)
            const wrapper = mountPanel(
                settingsForFov(h),
                astapStatus('G05'),
                databases(['G05', 'D80']),
            )
            await flushPromises()
            await wrapper.find('.fov-warning-btn').trigger('click')

            const text = wrapper.find('.alert-warning').text()
            expect(text).toContain('too narrow')
            expect(text).toContain('Switch to D80')
            // The warning message itself mentions G05 as the active DB,
            // but the suggestion should not suggest G05.
            expect(text).not.toContain('Switch to G05')
            expect(text).not.toContain('download G05')
        })

        it('handles empty available databases gracefully', async () => {
            const wrapper = mountPanel(
                settingsForFov(4),
                astapStatus('D80'),
                [],
            )
            await flushPromises()
            await wrapper.find('.fov-warning-btn').trigger('click')

            const text = wrapper.find('.alert-warning').text()
            expect(text).toContain('too narrow')
            expect(text).not.toContain('Switch')
            expect(text).not.toContain('download')
        })
    })
})

describe('PushToPanel – database selection', () => {
    beforeEach(() => {
        vi.clearAllMocks()
    })

    it('does not show database selector when only one database is installed', async () => {
        const wrapper = mountPanel(settingsForFov(100), astapStatus('D80'), databases(['D80']))
        await flushPromises()

        expect(wrapper.find('.database-select-row').exists()).toBe(false)
    })

    it('does not show database selector when no databases are installed', async () => {
        const status = {binary_installed: true, database_installed: false, installed_databases: [], ready: false}
        const wrapper = mountPanel(settingsForFov(100), status, databases())
        await flushPromises()

        expect(wrapper.find('.database-select-row').exists()).toBe(false)
    })

    it('shows database selector when multiple databases are installed', async () => {
        const wrapper = mountPanel(settingsForFov(100), astapStatus('D80', 'G05'), databases(['D80', 'G05']))
        await flushPromises()

        expect(wrapper.find('.database-select-row').exists()).toBe(true)
    })

    it('reflects the currently active database in the selector', async () => {
        const wrapper = mountPanel(settingsForFov(100), astapStatus('G05', 'D80'), databases(['D80', 'G05']))
        await flushPromises()

        const select = wrapper.find('.database-select-row select')
        expect(select.element.value).toBe('G05')
    })

    it('lists all installed databases as options', async () => {
        const wrapper = mountPanel(settingsForFov(100), astapStatus('D80', 'G05', 'W08'), databases(['D80', 'G05', 'W08']))
        await flushPromises()

        const options = wrapper.findAll('.database-select-row option')
        const values = options.map(o => o.element.value)
        expect(values).toContain('D80')
        expect(values).toContain('G05')
        expect(values).toContain('W08')
    })

    it('calls updatePushToConfig with the correct database path when selection changes', async () => {
        const wrapper = mountPanel(settingsForFov(100), astapStatus('D80', 'G05'), databases(['D80', 'G05']))
        await flushPromises()

        const select = wrapper.find('.database-select-row select')
        await select.setValue('G05')
        await flushPromises()

        expect(updatePushToConfig).toHaveBeenCalledWith({database_path: DB_CONFIGS.G05.database_path})
    })

    it('updates the FOV warning after switching to a database that covers the current FOV', async () => {
        // D80 active, FOV ≈ 6.02° is too wide for D80 (max 6°) → warning shown
        // After switching to G05 (3°–20°) which covers 6.02° → warning disappears
        const wrapper = mountPanel(
            settingsForFov(227),
            astapStatus('D80', 'G05'),
            databases(['D80', 'G05']),
        )
        await flushPromises()
        expect(wrapper.find('.fov-warning-btn').exists()).toBe(true)

        const select = wrapper.find('.database-select-row select')
        await select.setValue('G05')
        await flushPromises()

        expect(wrapper.find('.fov-warning-btn').exists()).toBe(false)
    })

    it('does not call updatePushToConfig when selecting the already-active database', async () => {
        const wrapper = mountPanel(settingsForFov(100), astapStatus('D80', 'G05'), databases(['D80', 'G05']))
        await flushPromises()

        const select = wrapper.find('.database-select-row select')
        await select.setValue('D80')
        await flushPromises()

        expect(updatePushToConfig).not.toHaveBeenCalled()
    })
})
