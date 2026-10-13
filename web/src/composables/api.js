/**
 * API client for Night Amplifier REST endpoints. Every call resolves with the envelope's
 * `data` or rejects with an `ApiError`; none returns an error value.
 */
import {API_BASE} from '../constants/endpoints.js'

const SERVER_UNAVAILABLE = 'Server unavailable. Please ensure the server is running.'

/**
 * A refused or failed request. `status` is the HTTP status, 0 when the server could not be
 * reached: 403 is a Pro-only route on a Community server (or a lapsed licence), 409 a
 * request the server's state refuses, 503 a busy one.
 */
export class ApiError extends Error {
    constructor(message, status = 0) {
        super(message)
        this.name = 'ApiError'
        this.status = status
    }
}

/** Whether a failure is the server declining a Pro feature it does not have. */
export function isProOnly(error) {
    return error?.status === 403
}

/**
 * For a caller to whom a Pro feature is optional: the 403 a Community server answers is
 * expected and swallowed, anything else is logged rather than lost.
 */
export function ignoreProOnly(error, what) {
    if (!isProOnly(error)) console.warn(`${what} failed:`, error?.message ?? error)
}

/**
 * Send one request. `body` goes as JSON, and only then with a Content-Type; `query` is
 * encoded onto the URL, skipping `null`/`undefined` values.
 */
async function send(path, {method = 'GET', query, body} = {}) {
    const init = {method, cache: 'no-store'}
    if (body !== undefined) {
        init.headers = {'Content-Type': 'application/json'}
        init.body = JSON.stringify(body)
    }
    try {
        return await fetch(withQuery(`${API_BASE}${path}`, query), init)
    } catch {
        throw new ApiError(SERVER_UNAVAILABLE)
    }
}

function withQuery(url, query) {
    const params = new URLSearchParams()
    for (const [key, value] of Object.entries(query ?? {})) {
        if (value != null) params.append(key, String(value))
    }
    const encoded = params.toString()
    return encoded ? `${url}?${encoded}` : url
}

/** A JSON route: unwraps the server's `{success, data, error}` envelope. */
async function request(path, options) {
    const response = await send(path, options)
    const text = await response.text()
    // A proxy with no server behind it answers with an empty body.
    if (!text) throw new ApiError(SERVER_UNAVAILABLE, response.status)

    let envelope
    try {
        envelope = JSON.parse(text)
    } catch {
        throw new ApiError(
            'Server returned invalid response. Please ensure the server is running.',
            response.status
        )
    }
    if (!envelope.success) throw new ApiError(envelope.error || 'API request failed', response.status)
    return envelope.data
}

const get = (path, query) => request(path, {query})
const post = (path, body) => request(path, {method: 'POST', body})
const put = (path, body) => request(path, {method: 'PUT', body})
const del = (path) => request(path, {method: 'DELETE'})

// ============================================================================
// Capabilities
// ============================================================================

/**
 * Get server capabilities (Pro features detection)
 * @returns {Promise<Capabilities>}
 */
export async function getCapabilities() {
    return get('/capabilities')
}

/**
 * The AI denoiser's hardware benchmark and where the network runs, resolved for the
 * saved "AI compute" choice.
 * @returns {Promise<object>}
 */
export async function getAiCompute() {
    return get('/ai-compute')
}

/**
 * Measure the hardware again: forget this computer's result and the units recorded as
 * crashing. Refused (409) while a capture runs.
 * @returns {Promise<object>} the report as it stands
 */
export async function remeasureAiCompute() {
    return post('/ai-compute/benchmark')
}

// ============================================================================
// Eyepiece
// ============================================================================

/** How long to wait before asking a busy server again. Matches its `Retry-After`. */
export const SNAPSHOT_RETRY_MS = 2000

/** How long to keep retrying a busy server before giving up. */
export const SNAPSHOT_RETRY_WINDOW_MS = 15000

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

/** Used when the server sends no `Content-Disposition` name of its own. */
const SNAPSHOT_FALLBACK_FILENAME = 'eyepiece.png'

/**
 * Read the download's name out of a `Content-Disposition` header.
 *
 * The name has to be lifted off the response and carried with the bytes: once
 * `fetch` has read the body, the blob built from it has no headers left, so the
 * `<a download>` that saves it cannot get the server's timestamped name any other
 * way — and every save would land on one filename instead.
 */
function snapshotFilename(header) {
    return /filename="([^"]+)"/i.exec(header ?? '')?.[1] ?? SNAPSHOT_FALLBACK_FILENAME
}

/**
 * Fetch the current eyepiece frame as a PNG, named by the server — not routed through
 * `request()` (parses JSON). One render at a time (hundreds of MB at native res); 503
 * means retry on that cadence, 404 means nothing's rendered yet — don't retry that.
 *
 * @param {boolean} circular - Round eyepiece image, or the uncropped frame.
 * @returns {Promise<{blob: Blob, filename: string}>}
 */
export async function fetchEyepieceSnapshot(circular) {
    const giveUpAt = Date.now() + SNAPSHOT_RETRY_WINDOW_MS

    for (;;) {
        const response = await send('/eyepiece/snapshot', {query: {circular}})

        if (response.ok) {
            return {
                blob: await response.blob(),
                filename: snapshotFilename(response.headers.get('content-disposition')),
            }
        }
        if (response.status === 404) throw new ApiError('No frame to download yet.', 404)
        if (response.status !== 503) {
            throw new ApiError(`Download failed (${response.status}).`, response.status)
        }

        // Busy. Only wait if the whole wait still fits inside the window, so the
        // last attempt is made before it closes rather than after.
        if (Date.now() + SNAPSHOT_RETRY_MS > giveUpAt) {
            throw new ApiError('Server busy rendering another download. Try again shortly.', 503)
        }
        await sleep(SNAPSHOT_RETRY_MS)
    }
}

// ============================================================================
// Capture Control
// ============================================================================

/**
 * Start the camera in `role`: the imaging camera's capture session, or the guide
 * camera's free-running loop.
 * @param {string} [cameraId] - Optional camera ID
 * @param {'main'|'guide'} [role] - Which camera to start
 */
export async function startCapture(cameraId = null, role = 'main') {
    const body = {}
    if (cameraId) body.camera_id = cameraId
    if (role !== 'main') body.role = role
    return post('/capture/start', body)
}

/**
 * Stop the camera in `role`. See {@link startCapture}.
 *
 * The imaging camera sends no body, which is the shape every release before roles
 * existed used and the shape the server still reads as "the imaging camera".
 * @param {'main'|'guide'} [role] - Which camera to stop
 */
export async function stopCapture(role = 'main') {
    return post('/capture/stop', role === 'main' ? undefined : {role})
}

// ============================================================================
// Settings
// ============================================================================

/**
 * Get current settings
 * @returns {Promise<Settings>}
 */
export async function getSettings() {
    return get('/settings')
}

/**
 * Update settings
 * @param {Partial<Settings>} settings - Settings to update
 * @returns {Promise<Settings>}
 */
export async function updateSettings(settings) {
    return post('/settings', settings)
}

/**
 * Get available stacking types
 * @returns {Promise<StackingType[]>}
 */
export async function getStackingTypes() {
    return get('/settings/stacking-types')
}

// ============================================================================
// Cameras
// ============================================================================

const cameraPath = (cameraId, action) => `/cameras/${encodeURIComponent(cameraId)}/${action}`

/**
 * List available cameras
 * @returns {Promise<Camera[]>}
 */
export async function listCameras() {
    return get('/cameras')
}

/**
 * Connect to a camera
 * @param {string} cameraId - Camera ID
 * @param {'main'|'guide'} [role] - Which position it takes. Omitted means the imaging camera.
 */
export async function connectCamera(cameraId, role = 'main') {
    return post(cameraPath(cameraId, 'connect'), {role})
}

/**
 * Disconnect a camera. A capture running on it is stopped first, and a cooled camera
 * warms up before it closes unless `skipWarmup` says to close it now.
 * @param {string} cameraId - Camera ID
 * @param {{skipWarmup?: boolean}} [options]
 * @returns {Promise<{message: string, camera_id: string, warming_up: boolean, warmup_remaining_s?: number}>}
 */
export async function disconnectCamera(cameraId, {skipWarmup = false} = {}) {
    return post(cameraPath(cameraId, 'disconnect'), skipWarmup ? {skip_warmup: true} : undefined)
}

/**
 * Choose the camera `/eyepiece` and `/eyepiece_quality` show — the Guide toggle on `/`.
 * Every client hears the change as `viewed_camera_changed`.
 * @param {'main'|'guide'} camera
 * @returns {Promise<{camera: 'main'|'guide'}>}
 */
export async function setViewedCamera(camera) {
    return put('/view/camera', {camera})
}

// ============================================================================
// Simulated Camera
// ============================================================================

/**
 * Configure simulated camera directory
 * @param {string} directory - Path to directory containing image files
 * @returns {Promise<SimulatorConfig>}
 */
export async function configureSimulator(directory) {
    return post('/simulator/configure', {directory})
}

/**
 * Get simulated camera configuration
 * @returns {Promise<SimulatorConfig>}
 */
export async function getSimulatorConfig() {
    return get('/simulator/config')
}

/**
 * Remove a simulated camera by index
 * @param {number} index - Index of the simulated camera to remove
 * @returns {Promise<SimulatorConfig>}
 */
export async function removeSimulatedCamera(index) {
    return del(`/simulator/${index}`)
}

// ============================================================================
// Push-To Navigation
// ============================================================================

/**
 * Get Push-To status
 * @returns {Promise<PushToStatus>}
 */
export async function getPushToStatus() {
    return get('/push-to/status')
}

/**
 * Search catalog for objects
 * @param {string} query - Search query
 * @param {number} [limit=20] - Max results
 * @returns {Promise<CatalogEntry[]>}
 */
export async function searchCatalog(query, limit = 20) {
    return get('/push-to/catalog/search', {query, limit})
}

/**
 * Set target by name/designation
 * @param {string} name - Object name or designation (e.g., "M31", "Andromeda")
 * @returns {Promise<TargetInfo>}
 */
export async function setTargetByName(name) {
    return post('/push-to/target', {name})
}

/**
 * Set target by coordinates
 * @param {number} ra - Right Ascension in degrees
 * @param {number} dec - Declination in degrees
 * @param {string} [name] - Optional name for the target
 * @returns {Promise<TargetInfo>}
 */
export async function setTargetByCoordinates(ra, dec, name = null) {
    return post('/push-to/target', {ra_degrees: ra, dec_degrees: dec, name})
}

/**
 * Clear current target
 */
export async function clearTarget() {
    return del('/push-to/target')
}

/**
 * Update Push-To configuration (FOV hint, database path)
 * @param {Object} config
 * @param {number} [config.fov_degrees] - Field of view hint in degrees
 * @param {string} [config.database_path] - Path to solver database
 * @returns {Promise<PushToStatus>}
 */
export async function updatePushToConfig(config) {
    return post('/push-to/config', config)
}

/**
 * Cancel current plate solving process
 */
export async function cancelPushToSolve() {
    return post('/push-to/cancel')
}

// ============================================================================
// ASTAP Installation
// ============================================================================

/**
 * Get ASTAP installation status
 * @returns {Promise<AstapStatus>}
 */
export async function getAstapStatus() {
    return get('/astap/status')
}

/**
 * Get available ASTAP database types
 * @returns {Promise<DatabaseType[]>}
 */
export async function getAstapDatabases() {
    return get('/astap/databases')
}

/**
 * Start ASTAP installation with selected databases
 * @param {string[]} [databaseTypes=['D80']] - Database types to install (e.g. ['D80', 'G05'])
 * @returns {Promise<{message: string}>}
 */
export async function installAstap(databaseTypes = ['D80']) {
    return post('/astap/install', {database_types: databaseTypes})
}

// ============================================================================
// Catalog Installation
// ============================================================================

/**
 * Get OpenNGC catalog installation status
 * @returns {Promise<CatalogStatus>}
 */
export async function getCatalogStatus() {
    return get('/catalog/status')
}

/**
 * Start target catalogs installation
 * @param {boolean} [includeStars=false] - Whether to include the HYG star database
 * @returns {Promise<{message: string}>}
 */
export async function installCatalog(includeStars = false) {
    return post('/catalog/install', {include_stars: includeStars})
}

// ============================================================================
// License & About
// ============================================================================

/**
 * Get the current license status and details
 * @returns {Promise<{active: boolean, details: object|null}>}
 */
export async function getLicenseStatus() {
    return get('/about/license')
}

/**
 * Update the Pro license token
 * @param {string} token
 * @returns {Promise<{active: boolean, details: object}>}
 */
export async function updateLicense(token) {
    return post('/about/license', {token})
}

/**
 * Get software licenses text (core and third party)
 * @returns {Promise<{version: string, core_license: string, third_party_licenses: string|null}>}
 */
export async function getSoftwareLicenses() {
    return get('/about/software-licenses')
}

// Type definitions are in api.types.js
