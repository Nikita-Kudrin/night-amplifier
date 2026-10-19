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

/**
 * The live view's zoom range. The floor is a share of the fitted scale, not a scale: the canvas
 * is the stream's native size, so a phone fits a 3008 px frame at ~0.12, and an absolute 0.5
 * floor made a pinch from there a 4x zoom no pinch could undo. `max` is screen pixels per
 * stream pixel.
 */
export const ZOOM_LIMITS = {
    minOfFit: 0.5,
    max: 5,
    /**
     * How far apart two fingers must be before a pinch is anchored: its zoom is the ratio
     * of their spread to this start, so fingers landing together jumped straight to `max`.
     */
    minPinchSpreadPx: 10,
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
