/**
 * Whole wire objects for tests, each overridable field by field. A test that hands a
 * component a partial object never sees the shape the server sends; these follow
 * `api.types.js`, which `fixtures.test.js` checks.
 */
import {DEFAULT_CAPABILITIES, defaultSettings} from '../constants'

/** @returns {import('../composables/api.types.js').Settings} */
export function settings(overrides = {}) {
    return {...defaultSettings(), ...overrides}
}

/** @returns {import('../composables/api.types.js').Capabilities} */
export function capabilities(overrides = {}) {
    return {...structuredClone(DEFAULT_CAPABILITIES), ...overrides}
}

/** Every Pro plugin active. */
export function proCapabilities() {
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

/** @returns {import('../composables/api.types.js').CameraInfo} */
export function cameraInfo(overrides = {}) {
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

/**
 * A camera list entry; connected as the imaging camera unless told otherwise.
 * @returns {import('../composables/api.types.js').Camera}
 */
export function camera(overrides = {}) {
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

/** @returns {import('../composables/api.types.js').AstapStatus} */
export function astapStatus(overrides = {}) {
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

/** @returns {import('../composables/api.types.js').DatabaseType} */
export function databaseType(overrides = {}) {
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

/** @returns {import('../composables/api.types.js').CatalogStatus} */
export function catalogStatus(overrides = {}) {
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

/** @returns {import('../composables/api.types.js').PushToStatus} */
export function pushToStatus(overrides = {}) {
    return {
        solver_ready: true,
        is_solving: false,
        current_target: null,
        last_position: null,
        direction: null,
        ...overrides,
    }
}

/** @returns {import('../composables/api.types.js').CatalogEntry} */
export function catalogEntry(overrides = {}) {
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
