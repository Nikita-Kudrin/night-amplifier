/**
 * JSDoc type definitions for Night Amplifier API responses.
 * These types are used across the frontend for documentation.
 */

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
 * @property {string} id - Database identifier (D80, G05, W08)
 * @property {string} description - Human-readable description
 * @property {number} min_fov_deg - Minimum field of view in degrees
 * @property {number} max_fov_deg - Maximum field of view in degrees
 * @property {string} size - Approximate download size
 * @property {boolean} installed - Whether this database is already installed
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
 * @property {boolean} is_solving - Whether plate solving is in progress
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
 * @property {string} catalog_type - Catalog type (Messier, NGC, IC, Star)
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
