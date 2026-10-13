/** The app shell: sidebar tabs, and what an unanswered `/api/capabilities` means. */
import type {Capabilities} from '../composables/api.types.js'

// Sidebar tabs: the header's compass and gear each show their panel alone
export const SIDEBAR_TABS = {
    MAIN: 'main',
    PUSH_TO: 'push-to',
    SETTINGS: 'settings',
} as const

/** Where "Upgrade to Pro" leads. */
export const PRO_UPGRADE_URL = 'https://skycontrast.com/software/night-amplifier-pro'

/**
 * `/api/capabilities` before it answers: Community only, every Pro control locked.
 * Frozen, since every component without a provided answer reads this one object.
 */
export const DEFAULT_CAPABILITIES = Object.freeze({
    has_pro: false,
    deep_sky: Object.freeze({
        advanced_rejection: false,
        rbf_background: false,
        saturation_boost: false,
        denoise: false,
        ai_denoise: false,
    }),
    planetary: Object.freeze({advanced_stacking: false}),
    push_to: Object.freeze({astap_solver: false}),
    comet: Object.freeze({pro_stacking: false}),
    debug_logging: false,
} satisfies Capabilities)
