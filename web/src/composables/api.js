/**
 * API client for Night Amplifier REST endpoints
 */

const BASE_URL = '/api'

/**
 * Make an API request
 * @param {string} endpoint - API endpoint
 * @param {object} options - Fetch options
 * @returns {Promise<object>} API response data
 */
async function request(endpoint, options = {}) {
    const url = `${BASE_URL}${endpoint}`
    const config = {
        headers: {
            'Content-Type': 'application/json',
            ...options.headers,
        },
        ...options,
    }

    if (options.body && typeof options.body === 'object') {
        config.body = JSON.stringify(options.body)
    }

    let response
    try {
        response = await fetch(url, config)
    } catch {
        throw new Error('Server unavailable. Please ensure the server is running.')
    }

    // Handle empty responses or non-JSON responses
    const text = await response.text()
    if (!text) {
        throw new Error('Server unavailable. Please ensure the server is running.')
    }

    let data
    try {
        data = JSON.parse(text)
    } catch {
        throw new Error('Server returned invalid response. Please ensure the server is running.')
    }

    if (!data.success) {
        throw new Error(data.error || 'API request failed')
    }

    return data.data
}

// ============================================================================
// Capabilities
// ============================================================================

/**
 * Get server capabilities (Pro features detection)
 * @returns {Promise<Capabilities>}
 */
export async function getCapabilities() {
    return request('/capabilities')
}

// ============================================================================
// Capture Control
// ============================================================================

/**
 * Start capture session
 * @param {string} [cameraId] - Optional camera ID
 */
export async function startCapture(cameraId = null) {
    return request('/capture/start', {
        method: 'POST',
        body: cameraId ? {camera_id: cameraId} : {},
    })
}

/**
 * Stop capture session
 */
export async function stopCapture() {
    return request('/capture/stop', {
        method: 'POST',
    })
}

/**
 * Get capture status
 * @returns {Promise<CaptureStatus>}
 */
export async function getCaptureStatus() {
    return request('/capture/status')
}

// ============================================================================
// Settings
// ============================================================================

/**
 * Get current settings
 * @returns {Promise<Settings>}
 */
export async function getSettings() {
    return request('/settings')
}

/**
 * Update settings
 * @param {Partial<Settings>} settings - Settings to update
 * @returns {Promise<Settings>}
 */
export async function updateSettings(settings) {
    return request('/settings', {
        method: 'POST',
        body: settings,
    })
}

/**
 * Get available stacking types
 * @returns {Promise<StackingType[]>}
 */
export async function getStackingTypes() {
    return request('/settings/stacking-types')
}

// ============================================================================
// Cameras
// ============================================================================

/**
 * List available cameras
 * @returns {Promise<Camera[]>}
 */
export async function listCameras() {
    return request('/cameras')
}

/**
 * Get camera info
 * @param {string} cameraId - Camera ID
 * @returns {Promise<CameraInfo>}
 */
export async function getCameraInfo(cameraId) {
    return request(`/cameras/${encodeURIComponent(cameraId)}`)
}

/**
 * Connect to a camera
 * @param {string} cameraId - Camera ID
 */
export async function connectCamera(cameraId) {
    return request(`/cameras/${encodeURIComponent(cameraId)}/connect`, {
        method: 'POST',
    })
}

/**
 * Disconnect from a camera
 * @param {string} cameraId - Camera ID
 */
export async function disconnectCamera(cameraId) {
    return request(`/cameras/${encodeURIComponent(cameraId)}/disconnect`, {
        method: 'POST',
    })
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
    return request('/simulator/configure', {
        method: 'POST',
        body: {directory},
    })
}

/**
 * Get simulated camera configuration
 * @returns {Promise<SimulatorConfig>}
 */
export async function getSimulatorConfig() {
    return request('/simulator/config')
}

/**
 * Remove a simulated camera by index
 * @param {number} index - Index of the simulated camera to remove
 * @returns {Promise<SimulatorConfig>}
 */
export async function removeSimulatedCamera(index) {
    return request(`/simulator/${index}`, {
        method: 'DELETE',
    })
}

// ============================================================================
// Push-To Navigation
// ============================================================================

/**
 * Get Push-To status
 * @returns {Promise<PushToStatus>}
 */
export async function getPushToStatus() {
    return request('/push-to/status')
}

/**
 * Search catalog for objects
 * @param {string} query - Search query
 * @param {number} [limit=20] - Max results
 * @returns {Promise<CatalogEntry[]>}
 */
export async function searchCatalog(query, limit = 20) {
    return request(`/push-to/catalog/search?query=${encodeURIComponent(query)}&limit=${limit}`)
}

/**
 * Set target by name/designation
 * @param {string} name - Object name or designation (e.g., "M31", "Andromeda")
 * @returns {Promise<{target: TargetInfo}>}
 */
export async function setTargetByName(name) {
    return request('/push-to/target', {
        method: 'POST',
        body: {name},
    })
}

/**
 * Set target by coordinates
 * @param {number} ra - Right Ascension in degrees
 * @param {number} dec - Declination in degrees
 * @param {string} [name] - Optional name for the target
 * @returns {Promise<{target: TargetInfo}>}
 */
export async function setTargetByCoordinates(ra, dec, name = null) {
    return request('/push-to/target', {
        method: 'POST',
        body: {ra_degrees: ra, dec_degrees: dec, name},
    })
}

/**
 * Clear current target
 */
export async function clearTarget() {
    return request('/push-to/target', {
        method: 'DELETE',
    })
}

/**
 * Get push direction to target
 * @returns {Promise<PushDirection>}
 */
export async function getPushDirection() {
    return request('/push-to/direction')
}

/**
 * Get all Messier objects
 * @returns {Promise<CatalogEntry[]>}
 */
export async function getMessierCatalog() {
    return request('/push-to/catalog/messier')
}

/**
 * Get NGC objects
 * @returns {Promise<CatalogEntry[]>}
 */
export async function getNGCCatalog() {
    return request('/push-to/catalog/ngc')
}

/**
 * Get IC objects
 * @returns {Promise<CatalogEntry[]>}
 */
export async function getICCatalog() {
    return request('/push-to/catalog/ic')
}

// ============================================================================
// ASTAP Installation
// ============================================================================

/**
 * Get ASTAP installation status
 * @returns {Promise<AstapStatus>}
 */
export async function getAstapStatus() {
    return request('/astap/status')
}

/**
 * Get available ASTAP database types
 * @returns {Promise<DatabaseType[]>}
 */
export async function getAstapDatabases() {
    return request('/astap/databases')
}

/**
 * Start ASTAP installation
 * @param {string} [databaseType='D20'] - Database type to install (D05, D20, D50)
 * @returns {Promise<{message: string}>}
 */
export async function installAstap(databaseType = 'D20') {
    return request('/astap/install', {
        method: 'POST',
        body: {database_type: databaseType},
    })
}

// ============================================================================
// Catalog Installation
// ============================================================================

/**
 * Get OpenNGC catalog installation status
 * @returns {Promise<CatalogStatus>}
 */
export async function getCatalogStatus() {
    return request('/catalog/status')
}

/**
 * Start OpenNGC catalog installation
 * @returns {Promise<{message: string}>}
 */
export async function installCatalog() {
    return request('/catalog/install', {
        method: 'POST',
    })
}

// ============================================================================
// Type definitions (for documentation)
// ============================================================================

/**
 * @typedef {object} AstapStatus
 * @property {boolean} binary_installed - Whether ASTAP CLI is installed
 * @property {string|null} binary_path - Path to ASTAP binary
 * @property {boolean} database_installed - Whether star database is installed
 * @property {string|null} database_path - Path to database directory
 * @property {string|null} database_type - Which database is installed (D05, D20, D50)
 * @property {boolean} ready - Whether system is ready for plate solving
 */

/**
 * @typedef {object} DatabaseType
 * @property {string} id - Database identifier (D05, D20, D50)
 * @property {string} name - Human-readable name
 * @property {string} description - Description including size and FOV range
 * @property {number} min_fov_degrees - Minimum field of view in degrees
 */

/**
 * @typedef {object} CaptureStatus
 * @property {string} state - Capture state (Idle, Starting, Capturing, Stopping, Error)
 * @property {number} frame_count - Total frames captured
 * @property {number} stacked_count - Successfully stacked frames
 * @property {number} rejected_count - Rejected frames
 * @property {string|null} last_error - Last error message
 * @property {number|null} started_at - Start timestamp (ms)
 * @property {number} exposure_us - Exposure time in microseconds
 * @property {number} gain - Current gain
 */

/**
 * @typedef {object} Settings
 * @property {number} exposure_us - Exposure time in microseconds
 * @property {number} gain - Gain value
 * @property {number} offset - Offset (black level)
 * @property {number} bin - Binning factor
 * @property {boolean} auto_stretch - Enable auto-stretch
 * @property {boolean} stacking - Enable stacking
 * @property {number} rejection_sigma - Sigma for rejection
 * @property {boolean} background_subtraction - Enable background subtraction
 */

/**
 * @typedef {object} Camera
 * @property {string} id - Camera ID
 * @property {string} name - Camera name
 * @property {boolean} connected - Connection status
 * @property {string} [provider] - Camera provider
 * @property {number} [index] - Provider index
 * @property {CameraInfo} info - Camera info
 */

/**
 * @typedef {object} CameraInfo
 * @property {string} id - Camera ID
 * @property {string} name - Camera name
 * @property {number} max_width - Maximum width
 * @property {number} max_height - Maximum height
 * @property {number} pixel_size_um - Pixel size in micrometers
 * @property {string} sensor_type - Sensor type
 * @property {boolean} has_cooler - Has cooler
 * @property {number} bit_depth - Bit depth
 * @property {number} min_exposure_us - Minimum exposure (us)
 * @property {number} max_exposure_us - Maximum exposure (us)
 * @property {number} min_gain - Minimum gain
 * @property {number} max_gain - Maximum gain
 */

/**
 * @typedef {object} SimulatorConfig
 * @property {boolean} configured - Whether simulator is configured
 * @property {string|null} directory - Configured directory path
 * @property {number|null} file_count - Number of image files found
 */

/**
 * @typedef {object} PushToStatus
 * @property {TargetInfo|null} target - Current target
 * @property {CoordinateInfo|null} current_position - Current telescope position
 * @property {PushDirection|null} direction - Direction to target
 * @property {boolean} solver_ready - Whether plate solver is ready
 */

/**
 * @typedef {object} TargetInfo
 * @property {string|null} name - Target name
 * @property {string|null} designation - Catalog designation
 * @property {number} ra_degrees - Right Ascension in degrees
 * @property {number} dec_degrees - Declination in degrees
 */

/**
 * @typedef {object} CoordinateInfo
 * @property {number} ra_degrees - Right Ascension in degrees
 * @property {number} dec_degrees - Declination in degrees
 */

/**
 * @typedef {object} PushDirection
 * @property {number} angle_degrees - Direction angle in degrees
 * @property {number} distance_degrees - Distance to target in degrees
 * @property {string} vertical_hint - Vertical direction hint (Up, Down, On Target)
 * @property {string} horizontal_hint - Horizontal direction hint (Left, Right, On Target)
 */

/**
 * @typedef {object} CatalogEntry
 * @property {string} designation - Catalog designation (e.g., "M31", "NGC 224")
 * @property {string} name - Common name
 * @property {string} catalog_type - Catalog type (Messier, NGC, IC)
 * @property {string} object_type - Object type (Galaxy, Nebula, etc.)
 * @property {string} constellation - Constellation
 * @property {number} ra_degrees - Right Ascension in degrees
 * @property {number} dec_degrees - Declination in degrees
 * @property {number|null} magnitude - Visual magnitude
 */

/**
 * @typedef {object} CatalogStatus
 * @property {boolean} installed - Whether the catalog is installed
 * @property {string|null} catalog_path - Path to catalog directory
 * @property {boolean} ngc_file_exists - Whether NGC.csv exists
 * @property {boolean} addendum_file_exists - Whether addendum.csv exists
 * @property {number|null} object_count - Number of objects in catalog
 */
