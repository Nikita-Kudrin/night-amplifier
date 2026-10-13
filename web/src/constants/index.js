/**
 * Application constants, one module per domain behind this barrel. Numbers the server owns
 * (defaults, clamp ranges) come from the generated `api.settings.ts`; what is here by hand
 * is the UI's own presentation: steps, labels, and ranges narrower than the server's.
 */

export * from './endpoints.js'
export * from './app.js'
export * from './capture.js'
export * from './stacking.js'
export * from './imageQuality.js'
export * from './eyepiece.js'
export * from './equipment.js'
export * from './view.js'
export * from './settingsDefaults.js'
export {HELP_TEXTS} from './help/index.js'
