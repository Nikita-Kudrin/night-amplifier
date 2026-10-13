/** Stacking: the stacking types, outlier rejection and frame weighting. */
import {STACKING_TYPE_VALUES} from '../composables/api.types.js'
import type {RejectionMethod, WeightingPreset} from '../composables/api.types.js'
import {serverLimits} from './serverSettings.js'
import {byName} from './tables.js'
import type {Option} from './tables.js'

/** `StackingType` (`api.types.ts`), by name. */
export const STACKING_TYPES = byName(STACKING_TYPE_VALUES)

/** Sigma clipping's threshold, in standard deviations from the pixel's mean. */
export const REJECTION_SIGMA_LIMITS = serverLimits('rejection_sigma', {step: 0.1})

/** Frame weighting presets for quality-based stacking. */
export const WEIGHTING_PRESET_OPTIONS = [
    {value: 'disabled', label: 'Disabled'},
    {value: 'balanced', label: 'Balanced'},
    {value: 'galaxies', label: 'Galaxies'},
    {value: 'nebulae', label: 'Nebulae'},
    {value: 'fwhm_only', label: 'FWHM Only'},
    {value: 'snr_only', label: 'SNR Only'},
] satisfies readonly Option<WeightingPreset>[]

/**
 * Outlier rejection methods for stacking. Min-Max is deliberately absent: it needs the
 * minimum and maximum of a sample set the live stack does not keep, so the backend
 * substitutes sigma clipping for it. Offering a choice that silently becomes another one
 * is worse than not offering it.
 */
export const REJECTION_METHOD_OPTIONS = [
    {value: 'None', label: 'None (Average)'},
    {value: 'SigmaClip', label: 'Sigma Clipping', pro: true},
    {value: 'WinsorizedSigmaClip', label: 'Winsorized', pro: true},
] satisfies readonly Option<Exclude<RejectionMethod, 'MinMax'>>[]

/** The smallest comet ROI side, in image pixels, worth tracking a nucleus in. */
export const COMET_ROI_MIN_PX = 20
