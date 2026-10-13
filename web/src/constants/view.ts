/** How the views behave: socket reconnects, zoom and fit, touch, save timing. */

// WebSocket reconnection settings
export const WS_RECONNECT = {
    // Base of an exponential backoff that doubles to `maxInterval` and then retries at
    // that rate forever — see `scheduleReconnect`. Starting at 1 s rather than the old
    // flat 3 s gets the common case (the server restarting under a kiosk display) back
    // on screen faster, while the ceiling keeps a long outage cheap.
    interval: 1000,
    maxInterval: 30000,
    pingInterval: 30000,
}

// Zoom limits for LiveView
/** The share of the container a fitted image fills. */
export const FIT_MARGIN = 0.95

export const ZOOM_LIMITS = {
    min: 0.5,
    max: 5,
    zoomInFactor: 1.2,
    zoomOutFactor: 0.8,
    wheelZoomIn: 1.1,
    wheelZoomOut: 0.9,
}

/**
 * How far a finger may travel and still be undecided: a tap (a pan that ends where it
 * started toggles the controls, one that travelled does not), or a gesture whose
 * intent — scroll or slide — is not known yet.
 */
export const TOUCH_SLOP_PX = 6

/** How long a settings control waits after the last edit before it saves. */
export const SETTINGS_DEBOUNCE_MS = 300
