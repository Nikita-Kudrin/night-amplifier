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
        cache: 'no-store',
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
 * Update Push-To configuration (FOV hint, database path)
 * @param {Object} config
 * @param {number} [config.fov_degrees] - Field of view hint in degrees
 * @param {string} [config.database_path] - Path to solver database
 * @returns {Promise<PushToStatus>}
 */
export async function updatePushToConfig(config) {
    return request('/push-to/config', {
        method: 'POST',
        body: config,
    })
}

/**
 * Cancel current plate solving process
 */
export async function cancelPushToSolve() {
    return request('/push-to/cancel', {
        method: 'POST',
    })
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
 * Start ASTAP installation with selected databases
 * @param {string[]} [databaseTypes=['D80']] - Database types to install (e.g. ['D80', 'G05'])
 * @returns {Promise<{message: string}>}
 */
export async function installAstap(databaseTypes = ['D80']) {
    return request('/astap/install', {
        method: 'POST',
        body: {database_types: databaseTypes},
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
// License & About
// ============================================================================

/**
 * Get the current license status and details
 * @returns {Promise<{active: boolean, details: object|null}>}
 */
export async function getLicenseStatus() {
    return request('/about/license')
}

/**
 * Update the Pro license token
 * @param {string} token 
 * @returns {Promise<{active: boolean, details: object}>}
 */
export async function updateLicense(token) {
    return request('/about/license', {
        method: 'POST',
        body: { token }
    })
}

/**
 * Get software licenses text (core and third party)
 * @returns {Promise<{core_license: string, third_party_licenses: string|null}>}
 */
export async function getSoftwareLicenses() {
    return request('/about/software-licenses')
}

// Type definitions are in api.types.js
