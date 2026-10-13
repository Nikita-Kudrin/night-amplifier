/** Stacking: the stacking types, outlier rejection and frame weighting. */
import {serverLimits} from './serverSettings.js'

/** `StackingType` (`api.types.ts`), by name. */
export const STACKING_TYPES = Object.freeze({
    DEEP_SKY: 'deep_sky',
    PLANETARY: 'planetary',
    COMET: 'comet',
})

/** Sigma clipping's threshold, in standard deviations from the pixel's mean. */
export const REJECTION_SIGMA_LIMITS = serverLimits('rejection_sigma', {step: 0.1})

/**
 * Frame weighting presets for quality-based stacking.
 * @type {{value: import('../composables/api.types.js').WeightingPreset, label: string}[]}
 */
export const WEIGHTING_PRESET_OPTIONS = [
    {value: 'disabled', label: 'Disabled'},
    {value: 'balanced', label: 'Balanced'},
    {value: 'galaxies', label: 'Galaxies'},
    {value: 'nebulae', label: 'Nebulae'},
    {value: 'fwhm_only', label: 'FWHM Only'},
    {value: 'snr_only', label: 'SNR Only'},
]

/**
 * Outlier rejection methods for stacking. Min-Max is deliberately absent: it needs the
 * minimum and maximum of a sample set the live stack does not keep, so the backend
 * substitutes sigma clipping for it. Offering a choice that silently becomes another one
 * is worse than not offering it.
 * @type {{value: import('../composables/api.types.js').RejectionMethod, label: string, pro?: boolean}[]}
 */
export const REJECTION_METHOD_OPTIONS = [
    {value: 'None', label: 'None (Average)'},
    {value: 'SigmaClip', label: 'Sigma Clipping', pro: true},
    {value: 'WinsorizedSigmaClip', label: 'Winsorized', pro: true},
]

/** The smallest comet ROI side, in image pixels, worth tracking a nucleus in. */
export const COMET_ROI_MIN_PX = 20
