/** The picture: stretch, background, saturation, sensor corrections, denoise, resolutions. */
import type {
    BackgroundExtractionAlgorithm,
    Resolution,
    StretchAggressiveness,
} from '../composables/api.types.js'
import {serverLimits} from './serverSettings.js'
import type {Option} from './tables.js'

/** Color Intensity. */
export const AUTO_STRETCH_INTENSITY = serverLimits('auto_stretch_intensity', {step: 0.05})

export const STRETCH_AGGRESSIVENESS_OPTIONS = [
    {value: 'high', label: 'Nebulae'},
    {value: 'medium', label: 'Deep Sky'},
    {value: 'low', label: 'Star Fields'},
] satisfies readonly Option<StretchAggressiveness>[]

export const BACKGROUND_ALGORITHM_OPTIONS = [
    {value: 'grid_bilinear', label: 'Grid (Fast)'},
    {value: 'rbf', label: 'RBF (Quality)', pro: true},
] satisfies readonly Option<BackgroundExtractionAlgorithm>[]

/** Shadow saturation boost strength; the slider starts at 10%. */
export const SATURATION_BOOST_LIMITS = serverLimits('saturation_boost_strength', {min: 0.1, step: 0.05})

/**
 * How far above its brightest same-colour neighbour a sample must sit to be treated as a
 * hot pixel; the server enforces the range (`HOT_PIXEL_SIGMA_RANGE`).
 */
export const HOT_PIXEL_SIGMA_LIMITS = serverLimits('sensor_correction.hot_pixel_sigma', {step: 0.5})

/** How far the chroma planes move toward the guided-filter result: a blend, so 100% is the whole way. */
export const DENOISE_CHROMA_STRENGTH_LIMITS = serverLimits('denoise.chroma_strength', {step: 0.05})

/**
 * Multiplier on the wavelet thresholds for the mid scales. Stops at 100%, the tuned value
 * and the point past which the filter stops helping: above it the mottle moves out past
 * the transform's reach instead of going away, and stars gain a visible ring.
 */
export const DENOISE_LUMA_STRENGTH_LIMITS = serverLimits('denoise.luma_strength', {step: 0.05})

export const BACKGROUND_GRAIN_LIMITS = serverLimits('denoise.background_grain', {step: 0.05})

/**
 * Local contrast on the target's own structure. 50% is the default the Pro plugin's
 * measurements passed on every test target; 100% is where the first faint ring shows.
 */
export const DETAIL_LIMITS = serverLimits('denoise.detail', {step: 0.05})

/**
 * How much sensor resolution the preview pipeline may bin away before it runs. Fixed for
 * the whole capture session — see the Rust `PreviewResolution`.
 */
export const PREVIEW_RESOLUTION_OPTIONS = [
    {value: 'native', label: 'Native'},
    {value: 'uhd2160', label: '4K'},
    {value: 'qhd1440', label: '1440p'},
    {value: 'hd1080', label: '1080p'},
] satisfies readonly Option<Resolution>[]

// The one JPEG size every client of `/` and `/eyepiece` receives — see the Rust
// `Resolution`. Same choices as Processing Resolution.
export const STREAMING_RESOLUTION_OPTIONS = PREVIEW_RESOLUTION_OPTIONS
