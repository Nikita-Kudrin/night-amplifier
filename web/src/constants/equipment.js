/** Cameras and optics: camera phases, cooler, dew heater, telescope, simulator. */
import {serverLimits} from './serverSettings.js'

/** `CameraPhase` (`api.types.js`), by name. */
export const CAMERA_PHASES = Object.freeze({
    DISCONNECTED: 'disconnected',
    IDLE: 'idle',
    PRECOOLING: 'precooling',
    CAPTURING: 'capturing',
    GUIDING: 'guiding',
    WARMING_UP: 'warming_up',
    RECOVERING: 'recovering',
})

/**
 * The cooler setpoint's range when the camera does not advertise its own, inside what the
 * server takes; the server's default is no setpoint at all, so the UI picks one.
 */
export const COOLER_TEMP_LIMITS = Object.freeze({
    min: -40,
    max: 20,
    step: 1,
    default: -10,
})

/** How close to its target the sensor must be for the cooler to read "Stable", in °C. */
export const COOLER_STABLE_BAND_C = 0.5

/** Dew heater power, in percent of full. */
export const DEW_HEATER_POWER_LIMITS = serverLimits('dew_heater_power', {step: 1})

/** The telescope fields' inputs: focal length in mm, Barlow/reducer factor, pixel size in µm. */
export const TELESCOPE_LIMITS = Object.freeze({
    focalLength: Object.freeze({min: 50, max: 10000}),
    barlow: Object.freeze({min: 0.1, max: 5.0, step: 0.05, default: 1.0}),
    pixelSize: Object.freeze({min: 0.5, max: 20.0, step: 0.01}),
})

/** How many images the simulator keeps in memory; the slider stops at 20. */
export const SIMULATED_PRELOAD_LIMITS = serverLimits('simulated_preload_images', {max: 20, step: 1})
