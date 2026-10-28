/**
 * Whole wire objects for tests, each overridable field by field. A test that hands a
 * component a partial object never sees the shape the server sends; each of these is
 * typed by the `api.types.ts` answer it stands in for.
 */
import {DEFAULT_CAPABILITIES, defaultSettings, STACKING_TYPES} from '../constants'
import {STACKING_TYPE_VALUES} from '../composables/api.types'
import type {ClientPushDirection} from '../composables/events/pushToEvents'
import type {
    AiComputeReport,
    AstapStatus,
    Camera,
    CameraInfo,
    Capabilities,
    CatalogEntry,
    CatalogStatus,
    DatabaseType,
    Settings,
    StackingType,
    StackingTypeInfo,
} from '../composables/api.types'

/** Settings as a test gives them: any field, and a group with only the fields it is about. */
export type SettingsOverrides = {
    [K in keyof Settings]?: Settings[K] extends object ? Partial<Settings[K]> : Settings[K]
}

const isGroup = (value: unknown): value is object =>
    value !== null && typeof value === 'object' && !Array.isArray(value)

/** The server's default settings with `overrides` over them, a group's fields over its own. */
export function settings(overrides: SettingsOverrides = {}): Settings {
    const merged: Record<string, unknown> = {...defaultSettings()}
    for (const [key, value] of Object.entries(overrides)) {
        const group = merged[key]
        merged[key] = isGroup(value) && isGroup(group) ? {...group, ...value} : value
    }
    // The defaults, each field (or a group's field) replaced by one of its own type.
    return merged as unknown as Settings
}

export function capabilities(overrides: Partial<Capabilities> = {}): Capabilities {
    return {...structuredClone(DEFAULT_CAPABILITIES), ...overrides}
}

/** Every Pro plugin active. */
export function proCapabilities(): Capabilities {
    return {
        has_pro: true,
        deep_sky: {
            advanced_rejection: true,
            rbf_background: true,
            saturation_boost: true,
            denoise: true,
            ai_denoise: true,
        },
        planetary: {advanced_stacking: true},
        push_to: {astap_solver: true},
        comet: {pro_stacking: true},
        debug_logging: false,
    }
}

/** A server with no unit to run the AI denoiser on, so nothing to benchmark. */
export function aiComputeReport(overrides: Partial<AiComputeReport> = {}): AiComputeReport {
    return {
        generation: 0,
        state: 'unavailable',
        progress: null,
        rungs: [],
        auto: null,
        effective: null,
        notice: null,
        ...overrides,
    }
}

export function cameraInfo(overrides: Partial<CameraInfo> = {}): CameraInfo {
    return {
        id: 'zwo_0',
        name: 'ASI533MC Pro',
        max_width: 3008,
        max_height: 3008,
        pixel_size_x_um: 3.76,
        pixel_size_y_um: 3.76,
        sensor_type: 'Color',
        has_cooler: false,
        has_dew_heater: false,
        bit_depth: 14,
        min_exposure_us: 32,
        max_exposure_us: 2000000000,
        min_gain: 0,
        max_gain: 450,
        sensor_modes: [],
        ...overrides,
    }
}

/** A camera list entry; connected as the imaging camera unless told otherwise. */
export function camera(overrides: Partial<Omit<Camera, 'info'>> & {info?: Partial<CameraInfo>} = {}): Camera {
    const {info, ...rest} = overrides
    const id = rest.id ?? 'zwo_0'
    const name = rest.name ?? 'ASI533MC Pro'
    return {
        id,
        name,
        connected: true,
        provider: 'ZWO',
        index: 0,
        role: 'main',
        phase: 'idle',
        ...rest,
        info: cameraInfo({id, name, ...info}),
    }
}

export function astapStatus(overrides: Partial<AstapStatus> = {}): AstapStatus {
    return {
        binary_installed: true,
        binary_path: '/opt/astap/astap_cli',
        database_installed: true,
        database_path: '/opt/astap/d80',
        database_type: 'D80',
        installed_databases: [{id: 'D80', database_path: '/opt/astap/d80', min_fov_deg: 0.15, max_fov_deg: 6}],
        ready: true,
        ...overrides,
    }
}

export function databaseType(overrides: Partial<DatabaseType> = {}): DatabaseType {
    return {
        id: 'D80',
        description: 'General Purpose',
        min_fov_deg: 0.15,
        max_fov_deg: 6,
        size: '~1.3GB',
        installed: false,
        ...overrides,
    }
}

export function catalogStatus(overrides: Partial<CatalogStatus> = {}): CatalogStatus {
    return {
        installed: true,
        catalog_path: '/opt/catalog',
        ngc_file_exists: true,
        addendum_file_exists: true,
        hyg_file_exists: false,
        object_count: 13957,
        ...overrides,
    }
}

export function catalogEntry(overrides: Partial<CatalogEntry> = {}): CatalogEntry {
    return {
        designation: 'NGC 1976',
        name: 'Orion Nebula',
        catalog_type: 'NGC',
        ra_degrees: 83.82,
        dec_degrees: -5.39,
        ra_string: '05h35m17s',
        dec_string: "-05°23'28\"",
        object_type: 'HII',
        magnitude: 4,
        constellation: 'Ori',
        messier: 'M42',
        ...overrides,
    }
}

type StackingTypeText = Pick<StackingTypeInfo, 'name' | 'description' | 'uses_star_registration'>

/** What `StackingType::info()` says of each type. */
const STACKING_TYPE_INFO: Record<StackingType, StackingTypeText> = {
    [STACKING_TYPES.DEEP_SKY]: {
        name: 'Deep Sky',
        description: 'Star-based registration with sigma clipping for nebulae, galaxies, and star clusters',
        uses_star_registration: true,
    },
    [STACKING_TYPES.PLANETARY]: {
        name: 'Planetary',
        description: 'Correlation-based alignment with quality selection for Moon, planets, and Sun',
        uses_star_registration: false,
    },
    [STACKING_TYPES.COMET]: {
        name: 'Comet',
        description: 'ROI centroid alignment with aggressive rejection for comets (stars rejected as outliers)',
        uses_star_registration: false,
    },
}

export function stackingTypeInfo(overrides: Partial<StackingTypeInfo> = {}): StackingTypeInfo {
    const id = overrides.id ?? STACKING_TYPES.DEEP_SKY
    return {
        id,
        ...STACKING_TYPE_INFO[id],
        supports_stacking: true,
        supports_quality_weighting: true,
        ...overrides,
    }
}

/** `GET /api/settings/stacking-types` as the server answers it. */
export function stackingTypes(): StackingTypeInfo[] {
    return STACKING_TYPE_VALUES.map((id) => stackingTypeInfo({id}))
}

/** A push direction as `events/pushToEvents.ts` hands it to the views: 10° due north. */
export function clientDirection(overrides: Partial<ClientPushDirection> = {}): ClientPushDirection {
    return {
        angleDeg: 0,
        distanceDeg: 10,
        directionHint: 'N',
        isClose: false,
        fovDeg: 0,
        solvedCamera: null,
        stale: false,
        ...overrides,
    }
}
