/** Capturing: exposure, gain and binning choices, the capture states and modes. */
import {CAPTURE_STATE_VALUES} from '../composables/api.types'
import type {CaptureState, RawFrameSaving} from '../composables/api.types'
import {serverDefault} from './serverSettings'
import {byName} from './tables'
import type {Option} from './tables'

// Exposure presets by unit
export const EXPOSURE_PRESETS = {
    us: [
        {label: '100', us: 100},
        {label: '500', us: 500},
        {label: '1000', us: 1000},
        {label: '2000', us: 2000},
        {label: '5000', us: 5000},
    ],
    ms: [
        {label: '5', us: 5000},
        {label: '10', us: 10000},
        {label: '20', us: 20000},
        {label: '40', us: 40000},
        {label: '50', us: 50000},
        {label: '100', us: 100000},
        {label: '200', us: 200000},
        {label: '300', us: 300000},
        {label: '400', us: 400000},
    ],
    s: [
        {label: '0.5', us: 500000},
        {label: '1', us: 1000000},
        {label: '2', us: 2000000},
        {label: '3', us: 3000000},
        {label: '5', us: 5000000},
        {label: '10', us: 10000000},
        {label: '15', us: 15000000},
        {label: '30', us: 30000000},
        {label: '60', us: 60000000},
    ],
}

// Binning options
export const BINNING_OPTIONS = [
    {value: 1, label: '1x1'},
    {value: 2, label: '2x2'},
    {value: 3, label: '3x3'},
    {value: 4, label: '4x4'},
] satisfies readonly Option<number>[]

/** Gain, before the camera reports its own range. */
export const GAIN_LIMITS = {
    min: 0,
    max: 500,
    default: serverDefault('gain'),
} as const

// Capture modes that can write raw frames, in the order the capture-mode switch shows them
export const RAW_FRAME_SAVING_MODES = [
    {key: 'live_view', label: 'Live view'},
    {key: 'wanderer', label: 'Wanderer'},
    {key: 'stacking', label: 'Stacking'},
    // Runs alongside the other three rather than instead of them, and only makes sense
    // with a guide camera attached. A diagnostic: the server saves guide frames only in
    // debug mode, ignoring a switch left on from one, so it is offered only then.
    {key: 'guide', label: 'Guide camera', requiresGuideCamera: true, requiresDebugMode: true},
] satisfies readonly {
    key: keyof RawFrameSaving
    label: string
    requiresGuideCamera?: boolean
    requiresDebugMode?: boolean
}[]

/**
 * `CaptureState` (`api.events.ts`), by name. `RECOVERING` is paused while the server
 * reopens a camera that dropped out; it resumes on its own.
 */
export const CAPTURE_STATES = byName(CAPTURE_STATE_VALUES)

/**
 * Whether a capture session is running, as far as the observer is concerned. A capture
 * paused for a camera reconnect still is: showing it as stopped is exactly the
 * disconnect flicker the server's quiet recovery exists to hide.
 */
export function isCaptureRunning(state: string): boolean {
    return state === CAPTURE_STATES.CAPTURING || state === CAPTURE_STATES.RECOVERING
}

/**
 * The capture panel's modes. Only the imaging camera has one: nothing the guide camera
 * produces is stacked, and offering the others for it would write the *imaging*
 * camera's mode, since the setting is global.
 */
export const CAPTURE_MODE_OPTIONS = [
    {value: 'off', label: 'Live view'},
    {value: 'wanderer', label: 'Wanderer'},
    {value: 'stacking', label: 'Stacking'},
] as const satisfies readonly Option<string>[]
export const GUIDE_CAPTURE_MODE_OPTIONS = [CAPTURE_MODE_OPTIONS[0]] as const

/**
 * How the status bar shows each capture state: `tone` names its indicator's style. A
 * capture paused for a reconnect reads as Capturing, as `isCaptureRunning` has it.
 */
export const CAPTURE_STATE_DISPLAY = {
    [CAPTURE_STATES.IDLE]: {tone: 'idle', label: 'Idle'},
    [CAPTURE_STATES.STARTING]: {tone: 'starting', label: 'Starting...'},
    [CAPTURE_STATES.CAPTURING]: {tone: 'capturing', label: 'Capturing'},
    [CAPTURE_STATES.RECOVERING]: {tone: 'capturing', label: 'Capturing'},
    [CAPTURE_STATES.STOPPING]: {tone: 'stopping', label: 'Stopping...'},
    [CAPTURE_STATES.ERROR]: {tone: 'error', label: 'Error'},
} as const satisfies Record<CaptureState, {tone: string; label: string}>

type CaptureStateDisplay = (typeof CAPTURE_STATE_DISPLAY)[CaptureState]

/**
 * `CAPTURE_STATE_DISPLAY`'s entry for `state`; a state this build does not know is Idle,
 * since an old cached tab can meet a newer server.
 */
export function captureStateDisplay(state: string): CaptureStateDisplay {
    const byState: Partial<Record<string, CaptureStateDisplay>> = CAPTURE_STATE_DISPLAY
    return byState[state] ?? CAPTURE_STATE_DISPLAY[CAPTURE_STATES.IDLE]
}
