/**
 * Application constants, one module per domain behind this barrel. Numbers the server owns
 * (defaults, clamp ranges) come from the generated `api.settings.ts`; what is here by hand
 * is the UI's own presentation: steps, labels, and ranges narrower than the server's.
 */

export * from './endpoints'
export * from './app'
export * from './capture'
export * from './stacking'
export * from './imageQuality'
export * from './eyepiece'
export * from './equipment'
export * from './view'
export * from './settingsDefaults'
export {HELP_TEXTS} from './help'
export type {Option} from './tables'
