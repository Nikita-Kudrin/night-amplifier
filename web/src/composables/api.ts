/**
 * API client for Night Amplifier REST endpoints. Every call resolves with the envelope's
 * `data` or rejects with an `ApiError`; none returns an error value.
 */
import {API_BASE} from '../constants/endpoints'
import type {
    AiComputeReport,
    AstapInstallRequest,
    AstapStatus,
    Camera,
    CameraRole,
    Capabilities,
    CatalogEntry,
    CatalogInstallRequest,
    CatalogStatus,
    ConfigureSimulatorRequest,
    ConnectCameraRequest,
    Coordinate,
    DatabaseType,
    DisconnectCameraRequest,
    Disconnection,
    LicenseStatus,
    Message,
    PushToConfigRequest,
    PushToStatus,
    SearchCatalogRequest,
    SetTargetRequest,
    Settings,
    SimulatorConfig,
    SoftwareLicenses,
    StackingTypeInfo,
    StartCaptureRequest,
    StopCaptureRequest,
    UpdateLicenseRequest,
    UpdateSettingsRequest,
    ViewedCamera,
} from './api.types'

const SERVER_UNAVAILABLE = 'Server unavailable. Please ensure the server is running.'
const TIMED_OUT = 'The server did not answer in time. Please ensure the server is running.'

/**
 * How long a request may take before it is abandoned. The settings store runs no read beside
 * a write, so one hung POST held every settings refresh behind it for good.
 */
export const REQUEST_TIMEOUT_MS = 15000

/**
 * For a request the server legitimately takes long over: a camera's SDK open, a stop that cuts
 * an in-flight sub short (up to 15 s), a native-size snapshot render, an install starting.
 */
export const LONG_REQUEST_TIMEOUT_MS = 120000

/** How much of a plain-text refusal (axum's 422 for a body it cannot read) the message keeps. */
const REASON_MAX_CHARS = 300

/** Gateway statuses: the proxy answered, the server behind it did not. */
const GATEWAY_STATUSES = new Set([502, 504])

/**
 * A refused or failed request. `status` is the HTTP status, 0 when the server could not be
 * reached or did not answer in time: 403 is a Pro-only route on a Community server (or a
 * lapsed licence), 409 a request the server's state refuses, 503 a busy one.
 */
export class ApiError extends Error {
    readonly status: number

    constructor(message: string, status = 0) {
        super(message)
        this.name = 'ApiError'
        this.status = status
    }
}

export function isApiError(error: unknown): error is ApiError {
    return error instanceof ApiError
}

/** Whether a failure is the server declining a Pro feature it does not have. */
export function isProOnly(error: unknown): boolean {
    return isApiError(error) && error.status === 403
}

/**
 * For a caller to whom a Pro feature is optional: the 403 a Community server answers is
 * expected and swallowed, anything else is logged rather than lost.
 */
export function ignoreProOnly(error: unknown, what: string): void {
    if (isProOnly(error)) return
    console.warn(`${what} failed:`, error instanceof Error ? error.message : error)
}

/**
 * Hardware fields of a settings patch, which land on one camera's profile
 * (`settings_service::apply`). They name the camera: binning without `camera_role`
 * rebinned the imaging camera while the guide camera was selected.
 */
type HardwareField =
    | 'exposure_us'
    | 'gain'
    | 'offset'
    | 'bin'
    | 'cooler_enabled'
    | 'cooler_fast_mode'
    | 'target_temp_c'
    | 'sensor_mode_override'
    | 'dew_heater_enabled'
    | 'dew_heater_power'

/** A hardware edit for `updateSettings`; leaving the role out does not compile. */
export type HardwarePatch = Pick<UpdateSettingsRequest, HardwareField> & {camera_role: CameraRole}

/**
 * Run `exchange(signal)` (a request and the read of its body) under a deadline; an abort
 * rejects as a timed-out `ApiError`, a network failure as an unreachable server.
 */
async function withTimeout<T>(
    timeoutMs: number,
    exchange: (signal: AbortSignal) => Promise<T>
): Promise<T> {
    const controller = new AbortController()
    const timer = setTimeout(() => controller.abort(), timeoutMs)
    try {
        return await exchange(controller.signal)
    } catch (e) {
        if (controller.signal.aborted) throw new ApiError(TIMED_OUT)
        if (e instanceof ApiError) throw e
        throw new ApiError(SERVER_UNAVAILABLE)
    } finally {
        clearTimeout(timer)
    }
}

type Query = Record<string, unknown>

interface RequestOptions {
    method?: string
    query?: Query
    body?: unknown
    timeoutMs?: number
}

/** The server's answer to a JSON route. */
type Envelope<T> = {success: true; data: T} | {success: false; error: string}

/**
 * Send one request. `body` goes as JSON, and only then with a Content-Type; `query` is
 * encoded onto the URL, skipping `null`/`undefined` values.
 */
function send(
    path: string,
    {method = 'GET', query, body}: Omit<RequestOptions, 'timeoutMs'> = {},
    signal?: AbortSignal
): Promise<Response> {
    const init: RequestInit = {method, cache: 'no-store', signal}
    if (body !== undefined) {
        init.headers = {'Content-Type': 'application/json'}
        init.body = JSON.stringify(body)
    }
    return fetch(withQuery(`${API_BASE}${path}`, query), init)
}

function withQuery(url: string, query: Query | undefined): string {
    const params = new URLSearchParams()
    for (const [key, value] of Object.entries(query ?? {})) {
        if (value != null) params.append(key, String(value))
    }
    const encoded = params.toString()
    return encoded ? `${url}?${encoded}` : url
}

/** A JSON route: unwraps the server's `{success, data, error}` envelope. */
function request<T>(
    path: string,
    {timeoutMs = REQUEST_TIMEOUT_MS, ...options}: RequestOptions = {}
): Promise<T> {
    return withTimeout(timeoutMs, async (signal) => {
        const response = await send(path, options, signal)
        return unwrap<T>(await response.text(), response)
    })
}

function unwrap<T>(text: string, {ok, status}: Pick<Response, 'ok' | 'status'>): T {
    // A proxy with no server behind it answers with an empty body.
    if (!text.trim()) throw new ApiError(SERVER_UNAVAILABLE, status)

    let envelope: Envelope<T>
    try {
        envelope = JSON.parse(text)
    } catch {
        throw new ApiError(plainTextReason(text, ok, status), status)
    }
    if (!envelope.success) throw new ApiError(envelope.error || 'API request failed', status)
    return envelope.data
}

/**
 * The message for an answer outside the envelope. A refusal the server wrote itself (axum's
 * 422 for a body it cannot read) keeps its reason: "ensure the server is running" sent the
 * user looking in the wrong place, and dropped what the server said.
 */
function plainTextReason(text: string, ok: boolean, status: number): string {
    if (GATEWAY_STATUSES.has(status)) return SERVER_UNAVAILABLE
    if (ok) return 'Server returned an invalid response.'
    const reason = text.trim()
    return reason.length > REASON_MAX_CHARS ? `${reason.slice(0, REASON_MAX_CHARS)}…` : reason
}

const get = <T>(path: string, query?: Query) => request<T>(path, {query})
const post = <T>(path: string, body?: unknown, timeoutMs?: number) =>
    request<T>(path, {method: 'POST', body, timeoutMs})
const put = <T>(path: string, body: unknown) => request<T>(path, {method: 'PUT', body})
const del = <T>(path: string) => request<T>(path, {method: 'DELETE'})

// ============================================================================
// Capabilities
// ============================================================================

/** Get server capabilities (Pro features detection) */
export async function getCapabilities(): Promise<Capabilities> {
    return get('/capabilities')
}

/**
 * The AI denoiser's hardware benchmark and where the network runs, resolved for the
 * saved "AI compute" choice.
 */
export async function getAiCompute(): Promise<AiComputeReport> {
    return get('/ai-compute')
}

/**
 * Measure the hardware again: forget this computer's result and the units recorded as
 * crashing. Refused (409) while a capture runs. Answers the report as it stands.
 */
export async function remeasureAiCompute(): Promise<AiComputeReport> {
    return post('/ai-compute/benchmark')
}

// ============================================================================
// Eyepiece
// ============================================================================

/** How long to wait before asking a busy server again. Matches its `Retry-After`. */
export const SNAPSHOT_RETRY_MS = 2000

/** How long to keep retrying a busy server before giving up. */
export const SNAPSHOT_RETRY_WINDOW_MS = 15000

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))

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
function snapshotFilename(header: string | null): string {
    return /filename="([^"]+)"/i.exec(header ?? '')?.[1] ?? SNAPSHOT_FALLBACK_FILENAME
}

/** A downloaded eyepiece frame and the name the server gave it. */
export interface EyepieceSnapshot {
    blob: Blob
    filename: string
}

/**
 * Fetch the current eyepiece frame as a PNG, named by the server — not routed through
 * `request()` (parses JSON). One render at a time (hundreds of MB at native res); 503
 * means retry on that cadence, 404 means nothing's rendered yet — don't retry that.
 *
 * `circular`: the round eyepiece image, or the uncropped frame.
 */
export async function fetchEyepieceSnapshot(circular: boolean): Promise<EyepieceSnapshot> {
    const giveUpAt = Date.now() + SNAPSHOT_RETRY_WINDOW_MS

    for (;;) {
        const snapshot = await withTimeout(LONG_REQUEST_TIMEOUT_MS, async (signal) => {
            const response = await send('/eyepiece/snapshot', {query: {circular}}, signal)
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
            return null
        })
        if (snapshot) return snapshot

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
 * camera's free-running loop. With no `cameraId` the server resolves the role's camera.
 */
export async function startCapture(
    cameraId: string | null = null,
    role: CameraRole = 'main'
): Promise<Message> {
    const body: StartCaptureRequest = {}
    if (cameraId) body.camera_id = cameraId
    if (role !== 'main') body.role = role
    return post('/capture/start', body, LONG_REQUEST_TIMEOUT_MS)
}

/**
 * Stop the camera in `role`. See {@link startCapture}.
 *
 * The imaging camera sends no body, which is the shape every release before roles
 * existed used and the shape the server still reads as "the imaging camera".
 */
export async function stopCapture(role: CameraRole = 'main'): Promise<Message> {
    const body = role === 'main' ? undefined : ({role} satisfies StopCaptureRequest)
    return post('/capture/stop', body, LONG_REQUEST_TIMEOUT_MS)
}

// ============================================================================
// Settings
// ============================================================================

/** Get current settings */
export async function getSettings(): Promise<Settings> {
    return get('/settings')
}

/**
 * Update settings: the fields `patch` names, every other one left as it is. A hardware
 * edit is a {@link HardwarePatch}. Answers the settings left in force.
 */
export async function updateSettings(patch: UpdateSettingsRequest): Promise<Settings> {
    return post('/settings', patch)
}

/** Get available stacking types */
export async function getStackingTypes(): Promise<StackingTypeInfo[]> {
    return get('/settings/stacking-types')
}

// ============================================================================
// Cameras
// ============================================================================

const cameraPath = (cameraId: string, action: string) =>
    `/cameras/${encodeURIComponent(cameraId)}/${action}`

/**
 * List available cameras. `discover: false` lists what the server's last enumeration
 * found, without the seconds of USB work a fresh one costs.
 */
export async function listCameras({discover = true}: {discover?: boolean} = {}): Promise<Camera[]> {
    return get('/cameras', discover ? undefined : {discover: false})
}

/**
 * Dismiss the warning automatic recovery left when it gave up on `role`'s camera, for every
 * client: the server restates it on each sync until a camera connects into the role.
 */
export async function dismissRecoveryFailure(role: CameraRole): Promise<Message> {
    return del(`/cameras/recovery/${encodeURIComponent(role)}`)
}

/** Connect to a camera in the position `role` names; omitted means the imaging camera. */
export async function connectCamera(cameraId: string, role: CameraRole = 'main'): Promise<Message> {
    const body = {role} satisfies ConnectCameraRequest
    return post(cameraPath(cameraId, 'connect'), body, LONG_REQUEST_TIMEOUT_MS)
}

/**
 * Disconnect a camera. A capture running on it is stopped first, and a cooled camera
 * warms up before it closes unless `skipWarmup` says to close it now.
 */
export async function disconnectCamera(
    cameraId: string,
    {skipWarmup = false}: {skipWarmup?: boolean} = {}
): Promise<Disconnection> {
    return post(
        cameraPath(cameraId, 'disconnect'),
        skipWarmup ? ({skip_warmup: true} satisfies DisconnectCameraRequest) : undefined,
        LONG_REQUEST_TIMEOUT_MS
    )
}

/**
 * Choose the camera `/eyepiece` and `/eyepiece_quality` show — the Guide toggle on `/`.
 * Every client hears the change as `viewed_camera_changed`.
 */
export async function setViewedCamera(camera: CameraRole): Promise<ViewedCamera> {
    return put('/view/camera', {camera} satisfies ViewedCamera)
}

// ============================================================================
// Simulated Camera
// ============================================================================

/** Add a simulated camera reading the image files in `directory` */
export async function configureSimulator(directory: string): Promise<SimulatorConfig> {
    const body = {directory} satisfies ConfigureSimulatorRequest
    return post('/simulator/configure', body, LONG_REQUEST_TIMEOUT_MS)
}

/** Get simulated camera configuration */
export async function getSimulatorConfig(): Promise<SimulatorConfig> {
    return get('/simulator/config')
}

/** Remove the simulated camera at `index` */
export async function removeSimulatedCamera(index: number): Promise<SimulatorConfig> {
    return del(`/simulator/${index}`)
}

// ============================================================================
// Push-To Navigation
// ============================================================================

/** Get Push-To status */
export async function getPushToStatus(): Promise<PushToStatus> {
    return get('/push-to/status')
}

/** Search the catalog for objects, at most `limit` of them */
export async function searchCatalog(query: string, limit = 20): Promise<CatalogEntry[]> {
    return get('/push-to/catalog/search', {query, limit} satisfies SearchCatalogRequest)
}

/**
 * Set target by name/designation (e.g., "M31", "Andromeda"). Answers the catalog entry
 * the name resolved to.
 */
export async function setTargetByName(name: string): Promise<CatalogEntry> {
    return post('/push-to/target', {name} satisfies SetTargetRequest)
}

/**
 * Set target by coordinates, in degrees. Sends no name: the server resolves one first, over
 * the coordinates.
 */
export async function setTargetByCoordinates(ra: number, dec: number): Promise<Coordinate> {
    return post('/push-to/target', {ra_degrees: ra, dec_degrees: dec} satisfies SetTargetRequest)
}

/** Clear current target */
export async function clearTarget(): Promise<Message> {
    return del('/push-to/target')
}

/** Update Push-To configuration (FOV hint, database path) */
export async function updatePushToConfig(config: PushToConfigRequest): Promise<PushToStatus> {
    return post('/push-to/config', config)
}

/** Cancel current plate solving process */
export async function cancelPushToSolve(): Promise<Message> {
    return post('/push-to/cancel')
}

// ============================================================================
// ASTAP Installation
// ============================================================================

/** Get ASTAP installation status */
export async function getAstapStatus(): Promise<AstapStatus> {
    return get('/astap/status')
}

/** Get available ASTAP database types */
export async function getAstapDatabases(): Promise<DatabaseType[]> {
    return get('/astap/databases')
}

/** Start ASTAP installation with the selected database types (e.g. ['D80', 'G05']) */
export async function installAstap(databaseTypes: string[] = ['D80']): Promise<Message> {
    const body = {database_types: databaseTypes} satisfies AstapInstallRequest
    return post('/astap/install', body, LONG_REQUEST_TIMEOUT_MS)
}

// ============================================================================
// Catalog Installation
// ============================================================================

/** Get OpenNGC catalog installation status */
export async function getCatalogStatus(): Promise<CatalogStatus> {
    return get('/catalog/status')
}

/** Start target catalogs installation, with the HYG star database if `includeStars` */
export async function installCatalog(includeStars = false): Promise<Message> {
    const body = {include_stars: includeStars} satisfies CatalogInstallRequest
    return post('/catalog/install', body, LONG_REQUEST_TIMEOUT_MS)
}

// ============================================================================
// License & About
// ============================================================================

/** Get the current license status and details */
export async function getLicenseStatus(): Promise<LicenseStatus> {
    return get('/about/license')
}

/** Update the Pro license token */
export async function updateLicense(token: string): Promise<LicenseStatus> {
    return post('/about/license', {token} satisfies UpdateLicenseRequest)
}

/** Get software licenses text (core and third party) */
export async function getSoftwareLicenses(): Promise<SoftwareLicenses> {
    return get('/about/software-licenses')
}
