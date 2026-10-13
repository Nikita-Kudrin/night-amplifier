import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {mount, flushPromises} from '@vue/test-utils'
import type {VueWrapper} from '@vue/test-utils'
import {ref} from 'vue'
import type {Ref} from 'vue'
import PushToPanel from './PushToPanel.vue'
import {createEventStreamStub} from '../test/eventStream.js'
import {camera, capabilities, catalogEntry, settings} from '../test/fixtures.js'
import type {AstapStatus, Camera, DatabaseType, InstalledDatabaseInfo, Settings} from '../composables/api.types.js'

// Mock the full API module used by PushToPanel and its composables
vi.mock('../composables/api.js', async (importOriginal) =>
    (await import('../test/mockApi.js')).mockApi(await importOriginal(), {
        getAstapStatus: vi.fn(),
        getAstapDatabases: vi.fn(),
        updateSettings: vi.fn().mockResolvedValue({}),
        updatePushToConfig: vi.fn().mockResolvedValue({}),
        searchCatalog: vi.fn().mockResolvedValue([]),
        setTargetByName: vi.fn(),
        setTargetByCoordinates: vi.fn(),
        clearTarget: vi.fn(),
        cancelPushToSolve: vi.fn(),
    }))

import {
    ApiError,
    getAstapStatus,
    getAstapDatabases,
    updatePushToConfig,
    searchCatalog,
    setTargetByName,
    setTargetByCoordinates,
} from '../composables/api.js'
import ManualCoordinates from './ManualCoordinates.vue'
import {fieldOfViewDeg} from '../utils/optics.js'
import {contextProvides} from '../composables/appContext.js'

// ─── FOV calculation helpers ──────────────────────────────────────────────────
//
// The frontend computes fovY = 2·atan(h·py / 2fl) (`utils/optics.js`), as Pro's solver does.
// These helpers produce settings that yield a specific, predictable fovY.

const FL = 10       // focal length mm
const PY = 4.63     // pixel size μm (square pixels)
const W = 100       // sensor width px (arbitrary — does not affect fovY)

/** The (fractional) sensor height whose fovY is exactly `fovDeg`. */
function exactHeightForFov(fovDeg: number) {
    return (2 * FL * Math.tan((fovDeg * Math.PI) / 360)) / (PY / 1000)
}

/** Sensor height (px) that produces approximately the target fovY. */
function heightForFov(targetFovDeg: number) {
    return Math.round(exactHeightForFov(targetFovDeg))
}

// The whole-pixel heights just inside and just outside a database's FOV range.
const insideMin = (fovDeg: number) => Math.ceil(exactHeightForFov(fovDeg))
const insideMax = (fovDeg: number) => Math.floor(exactHeightForFov(fovDeg))
const belowMin = (fovDeg: number) => insideMin(fovDeg) - 1
const aboveMax = (fovDeg: number) => insideMax(fovDeg) + 1

/** Build a settings ref that drives the telescope composable to the given fovY. */
function settingsForFov(sensorHeightPx: number) {
    return ref(settings({
        telescope: {
            focal_length_mm: FL,
            pixel_size_x_um: PY,
            pixel_size_y_um: PY,
            sensor_width_px: W,
            sensor_height_px: sensorHeightPx,
            barlow_coeff: 1.0,
        },
    }))
}

// ─── ASTAP status factories ───────────────────────────────────────────────────

const DB_CONFIGS = {
    D80: {id: 'D80', database_path: '/db/d80', min_fov_deg: 0.15, max_fov_deg: 6.0},
    G05: {id: 'G05', database_path: '/db/g05', min_fov_deg: 3.0, max_fov_deg: 20.0},
    W08: {id: 'W08', database_path: '/db/w08', min_fov_deg: 20.0, max_fov_deg: 80.0},
} satisfies Record<string, InstalledDatabaseInfo>

type DbId = keyof typeof DB_CONFIGS

function astapStatus(activeDbId: DbId, ...extraDbs: DbId[]): AstapStatus {
    return {
        binary_installed: true,
        database_installed: true,
        database_type: activeDbId,
        installed_databases: [DB_CONFIGS[activeDbId], ...extraDbs.map(id => DB_CONFIGS[id])],
        ready: true,
    }
}

// All available databases (from getAstapDatabases API) with numeric FOV fields
const ALL_DATABASES: DatabaseType[] = [
    {id: 'D80', description: 'General Purpose', min_fov_deg: 0.15, max_fov_deg: 6.0, size: '~1.3GB', installed: false},
    {id: 'G05', description: 'Camera Lenses', min_fov_deg: 3.0, max_fov_deg: 20.0, size: '~100MB', installed: false},
    {id: 'W08', description: 'Fisheye Lenses', min_fov_deg: 20.0, max_fov_deg: 80.0, size: '<1MB', installed: false},
]

/** Build a getAstapDatabases mock response with specified DBs marked as installed */
function databases(installedIds: string[] = []) {
    return ALL_DATABASES.map(db => ({...db, installed: installedIds.includes(db.id)}))
}

// ─── Component mounting ───────────────────────────────────────────────────────

/** `mockedAstapStatus` is what the status request answers, or the error it is refused with. */
function mountPanel(
    settings: Ref<Settings>,
    mockedAstapStatus: AstapStatus | ApiError,
    mockedDatabases = databases(),
    {cameras = [], selected = null}: {cameras?: Camera[]; selected?: string | null} = {},
) {
    if (mockedAstapStatus instanceof ApiError) vi.mocked(getAstapStatus).mockRejectedValue(mockedAstapStatus)
    else vi.mocked(getAstapStatus).mockResolvedValue(mockedAstapStatus)
    vi.mocked(getAstapDatabases).mockResolvedValue(mockedDatabases)

    return mount(PushToPanel, {
        global: {
            provide: contextProvides({
                settings,
                cameras: ref(cameras),
                selectedCamera: ref(selected),
                capabilities: ref(capabilities({has_pro: true, push_to: {astap_solver: true}})),
                eventStream: createEventStreamStub(),
            }),
            stubs: {
                AstapInstallOverlay: true,
                BaseProLock: true,
            },
        },
    })
}

/** Computed fovY for a sensor height using the frontend formula */
function computedFovY(h: number) {
    return fieldOfViewDeg(h, PY, FL)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

describe('PushToPanel – FOV warning', () => {
    beforeEach(() => {
        vi.clearAllMocks()
    })

    // ── No-warning conditions ────────────────────────────────────────────────

    describe('no warning shown when', () => {
        it('ASTAP status is refused (no plate solver plugin)', async () => {
            const wrapper = mountPanel(settingsForFov(100), new ApiError('Pro only', 403))
            await flushPromises()

            expect(wrapper.find('.fov-warning-btn').exists()).toBe(false)
        })

        it('ASTAP status has no database_type', async () => {
            const status: AstapStatus = {binary_installed: false, database_installed: false, installed_databases: [], ready: false}
            const wrapper = mountPanel(settingsForFov(100), status)
            await flushPromises()

            expect(wrapper.find('.fov-warning-btn').exists()).toBe(false)
        })

        it('database_type is not found in installed_databases', async () => {
            const status: AstapStatus = {
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
            const emptySettings = ref(settings()) // the server's default: no optics
            const wrapper = mountPanel(emptySettings, astapStatus('D80'))
            await flushPromises()

            expect(wrapper.find('.fov-warning-btn').exists()).toBe(false)
        })

        // ── Per-database in-range tests ─────────────────────────────────────

        type InRange = {db: DbId; h: number; label: string}
            & ({op?: undefined} | {op: 'gte'; minFov: number} | {op: 'lte'; maxFov: number})

        it.each<InRange>([
            {db: 'D80', h: insideMin(0.15), minFov: 0.15, op: 'gte', label: 'minimum'},
            {db: 'D80', h: heightForFov(2.5), label: 'mid-range (≈2.5°)'},
            {db: 'D80', h: insideMax(6), maxFov: 6.0, op: 'lte', label: 'maximum'},
            {db: 'G05', h: insideMin(3), minFov: 3.0, op: 'gte', label: 'minimum'},
            {db: 'G05', h: heightForFov(10), label: 'mid-range (≈10°)'},
            {db: 'G05', h: insideMax(20), maxFov: 20.0, op: 'lte', label: 'maximum'},
            {db: 'W08', h: insideMin(20), minFov: 20.0, op: 'gte', label: 'minimum'},
            {db: 'W08', h: heightForFov(50), label: 'mid-range (≈50°)'},
            {db: 'W08', h: insideMax(80), maxFov: 80.0, op: 'lte', label: 'maximum'},
        ])('$db: no warning at $label boundary', async (row) => {
            const {db, h} = row
            if (row.op === 'gte') expect(computedFovY(h)).toBeGreaterThanOrEqual(row.minFov)
            if (row.op === 'lte') expect(computedFovY(h)).toBeLessThanOrEqual(row.maxFov)
            const wrapper = mountPanel(settingsForFov(h), astapStatus(db))
            await flushPromises()
            expect(wrapper.find('.fov-warning-btn').exists()).toBe(false)
        })
    })

    // ── Warning conditions ───────────────────────────────────────────────────

    describe('warning shown when', () => {
        it.each([
            {db: 'D80', h: belowMin(0.15), threshold: 0.15, op: 'lt', direction: 'too narrow'},
            {db: 'D80', h: aboveMax(6), threshold: 6.0, op: 'gt', direction: 'too wide'},
            {db: 'G05', h: belowMin(3), threshold: 3.0, op: 'lt', direction: 'too narrow'},
            {db: 'G05', h: aboveMax(20), threshold: 20.0, op: 'gt', direction: 'too wide'},
            {db: 'W08', h: belowMin(20), threshold: 20.0, op: 'lt', direction: 'too narrow'},
            {db: 'W08', h: aboveMax(80), threshold: 80.0, op: 'gt', direction: 'too wide'},
        ] as const)('$db: shows warning icon when FOV is $direction', async ({db, h, threshold, op}) => {
            if (op === 'lt') expect(computedFovY(h)).toBeLessThan(threshold)
            if (op === 'gt') expect(computedFovY(h)).toBeGreaterThan(threshold)
            const wrapper = mountPanel(settingsForFov(h), astapStatus(db))
            await flushPromises()
            expect(wrapper.find('.fov-warning-btn').exists()).toBe(true)
        })

        it.each([
            {db: 'D80', h: belowMin(0.15), direction: 'too narrow'},
            {db: 'D80', h: aboveMax(6), direction: 'too wide'},
            {db: 'G05', h: belowMin(3), direction: 'too narrow'},
            {db: 'G05', h: aboveMax(20), direction: 'too wide'},
            {db: 'W08', h: belowMin(20), direction: 'too narrow'},
            {db: 'W08', h: aboveMax(80), direction: 'too wide'},
        ] as const)('$db: message says "$direction" and mentions database name', async ({db, h, direction}) => {
            const wrapper = mountPanel(settingsForFov(h), astapStatus(db))
            await flushPromises()
            await wrapper.find('.fov-warning-btn').trigger('click')
            const text = wrapper.find('.alert-warning').text()
            expect(text).toContain(direction)
            expect(text).toContain(db)
        })
    })

    // ── Boundary edge cases ──────────────────────────────────────────────────

    describe('boundary edge cases', () => {
        it('D80/G05 overlap: FOV 6° fits D80 but also G05 — correct active DB determines warning', async () => {
            const h = insideMax(6) // just under 6°, within both D80 and G05

            // Active DB = D80 → no warning (≤ D80 max 6°)
            const wrapperD80 = mountPanel(settingsForFov(h), astapStatus('D80'))
            await flushPromises()
            expect(wrapperD80.find('.fov-warning-btn').exists()).toBe(false)

            // Active DB = G05 → no warning (≥ G05 min 3°)
            const wrapperG05 = mountPanel(settingsForFov(h), astapStatus('G05'))
            await flushPromises()
            expect(wrapperG05.find('.fov-warning-btn').exists()).toBe(false)
        })

        it('D80/G05 overlap: FOV just over 6° is too wide for D80 but fine for G05', async () => {
            const h = aboveMax(6) // just over 6°

            // Active DB = D80 → warning (> D80 max 6°)
            const wrapperD80 = mountPanel(settingsForFov(h), astapStatus('D80'))
            await flushPromises()
            expect(wrapperD80.find('.fov-warning-btn').exists()).toBe(true)

            // Active DB = G05 → no warning (within G05 3°–20°)
            const wrapperG05 = mountPanel(settingsForFov(h), astapStatus('G05'))
            await flushPromises()
            expect(wrapperG05.find('.fov-warning-btn').exists()).toBe(false)
        })

        it('G05/W08 boundary: FOV just under 20° is fine for G05 but too narrow for W08', async () => {
            const h = insideMax(20) // just under 20°

            // Active DB = G05 → no warning (≤ G05 max 20°)
            const wrapperG05 = mountPanel(settingsForFov(h), astapStatus('G05'))
            await flushPromises()
            expect(wrapperG05.find('.fov-warning-btn').exists()).toBe(false)

            // Active DB = W08 → warning (< W08 min 20°)
            const wrapperW08 = mountPanel(settingsForFov(h), astapStatus('W08'))
            await flushPromises()
            expect(wrapperW08.find('.fov-warning-btn').exists()).toBe(true)
        })

        it('G05/W08 boundary: FOV just over 20° is too wide for G05 but fine for W08', async () => {
            const h = aboveMax(20) // just over 20°

            // Active DB = G05 → warning (> G05 max 20°)
            const wrapperG05 = mountPanel(settingsForFov(h), astapStatus('G05'))
            await flushPromises()
            expect(wrapperG05.find('.fov-warning-btn').exists()).toBe(true)

            // Active DB = W08 → no warning (≥ W08 min 20°)
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
            // Just above W08 max (80°)
            expect(computedFovY(aboveMax(80))).toBeGreaterThan(80.0)
            const wrapper = mountPanel(settingsForFov(aboveMax(80)), astapStatus('W08'))
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
            // Just above W08 max (80°)
            const wrapper = mountPanel(
                settingsForFov(aboveMax(80)),
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
        const status: AstapStatus = {binary_installed: true, database_installed: false, installed_databases: [], ready: false}
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

        const select = wrapper.find<HTMLSelectElement>('.database-select-row select')
        expect(select.element.value).toBe('G05')
    })

    it('lists all installed databases as options', async () => {
        const wrapper = mountPanel(settingsForFov(100), astapStatus('D80', 'G05', 'W08'), databases(['D80', 'G05', 'W08']))
        await flushPromises()

        const options = wrapper.findAll<HTMLOptionElement>('.database-select-row option')
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

describe('PushToPanel – catalog search', () => {
    const SEARCH_DEBOUNCE_MS = 300
    const M4 = catalogEntry({
        designation: 'NGC 6121',
        name: null,
        catalog_type: 'NGC',
        messier: 'M4',
        object_type: 'Globular Cluster',
        constellation: 'Scorpius',
    })

    // flushPromises schedules through setImmediate, which fake timers freeze
    const settle = () => vi.advanceTimersByTimeAsync(0)

    beforeEach(() => {
        vi.clearAllMocks()
        vi.useFakeTimers()
    })

    afterEach(() => {
        vi.useRealTimers()
    })

    async function mountSearchPanel() {
        const wrapper = mountPanel(settingsForFov(heightForFov(1)), astapStatus('D80'))
        await settle()
        return wrapper
    }

    async function typeKeys(wrapper: VueWrapper, ...values: string[]) {
        const input = wrapper.find<HTMLInputElement>('.search-input')
        for (const value of values) {
            await input.setValue(value)
        }
        await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS)
        await settle()
        return input
    }

    it('finds M4 typed one key at a time and targets the picked object without searching again', async () => {
        vi.mocked(searchCatalog).mockResolvedValue([M4])
        vi.mocked(setTargetByName).mockResolvedValue(M4)
        const wrapper = await mountSearchPanel()

        const input = await typeKeys(wrapper, 'M', 'M4')

        expect(searchCatalog).toHaveBeenCalledTimes(1)
        expect(searchCatalog).toHaveBeenCalledWith('M4')
        const item = wrapper.find('.search-result-item')
        expect(item.text()).toContain('NGC 6121')
        expect(item.find('.badge-messier').text()).toBe('M4')

        await item.trigger('click')
        await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS)
        await settle()

        expect(setTargetByName).toHaveBeenCalledWith('NGC 6121')
        expect(input.element.value).toBe('NGC 6121')
        expect(searchCatalog).toHaveBeenCalledTimes(1)
        expect(wrapper.find('.search-results').exists()).toBe(false)
    })

    it('says which name or identifier a result was found by', async () => {
        vi.mocked(searchCatalog).mockResolvedValue([catalogEntry({
            designation: 'NGC 6302',
            name: 'Bug Nebula',
            catalog_type: 'NGC',
            messier: null,
            matched_name: 'C 69',
            object_type: 'Planetary Nebula',
            constellation: 'Scorpius',
        })])
        const wrapper = await mountSearchPanel()

        await typeKeys(wrapper, 'C69')

        expect(wrapper.find('.result-matched').text()).toBe('Matched: C 69')
        expect(wrapper.find('.badge-messier').exists()).toBe(false)
    })

    it('shows no matched line when the designation or name matched', async () => {
        vi.mocked(searchCatalog).mockResolvedValue([M4])
        const wrapper = await mountSearchPanel()

        await typeKeys(wrapper, 'NGC 6121')

        expect(wrapper.find('.search-result-item').exists()).toBe(true)
        expect(wrapper.find('.result-matched').exists()).toBe(false)
    })
})

// Review of 4fc5053, R1: the Equipment fields edit `last_camera_name`'s profile, which
// with no camera selected is whichever camera was clicked last — often the guide. A
// heading naming nobody let the imaging scope's focal length land in the guide's profile.
describe('PushToPanel – Equipment heading', () => {
    const GUIDE_SCOPE = {
        focal_length_mm: 200,
        pixel_size_x_um: 2.9,
        pixel_size_y_um: 2.9,
        sensor_width_px: 1936,
        sensor_height_px: 1096,
        barlow_coeff: 1,
    }

    beforeEach(() => {
        vi.clearAllMocks()
    })

    it('names the camera whose profile it edits when no camera is selected', async () => {
        const scopes = ref(settings({
            telescope: GUIDE_SCOPE,
            camera_telescope_profiles: {'ASI120MM Mini': GUIDE_SCOPE},
            last_camera_name: 'ASI120MM Mini',
        }))
        const wrapper = mountPanel(scopes, astapStatus('D80'))
        await flushPromises()

        expect(wrapper.find('.equipment-header').text()).toContain('ASI120MM Mini')
    })

    it('names the selected camera once one is', async () => {
        const scopes = ref(settings({
            telescope: GUIDE_SCOPE,
            camera_telescope_profiles: {'ASI120MM Mini': GUIDE_SCOPE},
            last_camera_name: 'ASI120MM Mini',
        }))
        const imaging = camera({id: 'cam-a', name: 'ASI533MC Pro', role: 'main', connected: true,
            info: {name: 'ASI533MC Pro', pixel_size_x_um: 3.76, max_width: 3008, max_height: 3008}})
        const wrapper = mountPanel(scopes, astapStatus('D80'), databases(), {cameras: [imaging], selected: 'cam-a'})
        await flushPromises()

        expect(wrapper.find('.equipment-header').text()).toContain('Main · ASI533MC Pro')
    })
})

describe('PushToPanel – manual coordinates', () => {
    beforeEach(() => {
        vi.clearAllMocks()
    })

    // The server answers with the target itself; reading a `.target` off it blanked the
    // current target until the next event arrived.
    it('shows the target the server answers with', async () => {
        vi.mocked(setTargetByCoordinates).mockResolvedValue(
            {ra_degrees: 83.82, dec_degrees: -5.39, ra_string: '05h35m17s', dec_string: "-05°23'28\""})
        const wrapper = mountPanel(settingsForFov(heightForFov(1)), astapStatus('D80'))
        await flushPromises()

        const took = await wrapper.findComponent(ManualCoordinates).props('submit')({ra: 83.82, dec: -5.39})
        await flushPromises()

        expect(took).toBe(true)
        expect(setTargetByCoordinates).toHaveBeenCalledWith(83.82, -5.39)
        expect(wrapper.find('.current-target').text()).toContain('Custom')
    })

    it('keeps the fields when the server refuses', async () => {
        vi.mocked(setTargetByCoordinates).mockRejectedValue(new Error('Catalog not installed'))
        const wrapper = mountPanel(settingsForFov(heightForFov(1)), astapStatus('D80'))
        await flushPromises()

        const took = await wrapper.findComponent(ManualCoordinates).props('submit')({ra: 1, dec: 2})
        await flushPromises()

        expect(took).toBe(false)
        expect(wrapper.find('.current-target').exists()).toBe(false)
        expect(wrapper.text()).toContain('Catalog not installed')
    })
})

describe('PushToPanel – event stream', () => {
    // No stand-in for the server's socket: without one the panel does not mount.
    it('needs the provided event stream', () => {
        const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
        expect(() => mount(PushToPanel, {
            global: {provide: contextProvides({settings: ref(settings())}), stubs: {AstapInstallOverlay: true}},
        })).toThrow('PushToPanel needs the provided event stream')
        warn.mockRestore()
    })
})
