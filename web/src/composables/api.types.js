/**
 * JSDoc types of the server's JSON answers, generated from the Rust wire types by
 * `tests/api_types.rs`. Do not edit: change the Rust type, then run
 *   UPDATE_API_TYPES=1 cargo test --features api-schema --test api_types
 */

/**
 * What `GET`/`POST /api/settings` answer: every setting, in `settings.json`'s shape.
 * @typedef {object} Settings
 * @property {number} exposure_us - Exposure time in microseconds
 * @property {number} gain - Gain value
 * @property {number} offset - Offset (black level)
 * @property {number} bin - Binning factor
 * @property {boolean} auto_stretch - Enable auto-stretch for preview
 * @property {boolean} stacking - Enable live stacking
 * @property {number} rejection_sigma - Sigma for rejection during stacking
 * @property {RejectionMethod} rejection_method - Outlier rejection method. A fresh install and a file without the key both start on [`RejectionMethod::best_available`].
 * @property {boolean} background_subtraction - Enable background subtraction
 * @property {BackgroundExtractionAlgorithm} background_extraction_algorithm - Algorithm for background extraction (GridBilinear or RBF)
 * @property {RawFrameSaving} raw_frame_saving - Which capture modes write their raw frames to disk (FITS format)
 * @property {boolean} save_stacked_image - Enable saving stacked image to disk (FITS + PNG)
 * @property {StackingType} stacking_type - Stacking type (Deep Sky or Planetary)
 * @property {WeightingPreset} weighting_preset - Quality-based frame weighting preset for stacking
 * @property {StretchAggressiveness} stretch_aggressiveness - Auto stretch aggressiveness (Low, Medium, High)
 * @property {number} auto_stretch_intensity - Auto Stretch intensity multiplier (0.0 to 1.0, where 0.0 means no color boost, default 0.3)
 * @property {boolean} saturation_boost - Enable shadow saturation boost
 * @property {number} saturation_boost_strength - Shadow saturation boost strength (0.0-1.0)
 * @property {boolean} use_simulated_camera - Use simulated camera instead of a real one
 * @property {number} simulated_preload_images - Number of images to preload for simulated camera
 * @property {boolean} show_focus_image - Show the focus image when waiting for frames
 * @property {boolean} force_focus_image_now - Force showing the focus image even when the stream is active
 * @property {boolean} cooler_enabled - Whether the cooler should be active during capture (cooled cameras only)
 * @property {number|null} [target_temp_c] - Target sensor temperature in Celsius (None means "no target set")
 * @property {boolean} cooler_fast_mode - Bypass the 5 °C/min ramp and cool/warm as fast as the hardware allows. Defeats sensor-stress / condensation protections — user-opt-in only.
 * @property {DualSamplingMode|null} [sensor_mode_override] - Manual override for camera sensor mode. None means "derive from stacking_type".
 * @property {AlignmentRoi|null} [comet_roi] - Region of interest for comet nucleus tracking
 * @property {AlignmentRoi|null} [planetary_roi] - Region of interest for planetary alignment
 * @property {boolean} planetary_auto_tracking - Enable auto tracking of planetary ROI
 * @property {boolean} planetary_multi_point_alignment - Enable multi-point alignment for planetary (Pro only)
 * @property {boolean} dew_heater_enabled - Whether anti-dew heater is enabled
 * @property {number} dew_heater_power - Anti-dew heater power level (0-100)
 * @property {boolean} wanderer_mode - Enable "Wanderer" mode for automatic stack reset on movement
 * @property {boolean} auto_reconnect - Reopen the camera automatically after it drops out mid-session (a USB stall or an unplug), instead of leaving the session dead until someone clicks Connect.
 * @property {boolean} auto_resume_capture - After an automatic reconnect, resume the capture that was interrupted — same mode, same settings, same stack. Without this the camera comes back but the session does not.
 * @property {SensorCorrectionSettings} sensor_correction - Corrections applied to the raw sensor mosaic, before demosaic
 * @property {DenoiseSettings} denoise - Spatial denoising, applied at stream resolution inside the encoders
 * @property {Resolution} preview_resolution - How much sensor resolution the preview pipeline may bin away before it runs
 * @property {Resolution} streaming_resolution - The JPEG size every `/` and `/eyepiece` client receives
 * @property {EyepieceSettings} eyepiece - Eyepiece view settings
 * @property {TelescopeSettings} telescope - Telescope and camera parameters for FOV calculation
 * @property {Object<string, TelescopeSettings>} [camera_telescope_profiles] - Per-camera telescope profiles keyed by camera name
 * @property {Object<string, CameraCaptureProfile>} [camera_profiles] - Per-camera capture profiles keyed by `"{provider}/{model_name}"`. Holds the seven hardware-specific fields so switching between cameras doesn't leak stale values (e.g. cooler=true from a cooled camera into an uncooled one).
 * @property {CameraCaptureProfile} guide_camera - The guide camera's live hardware values.
 * @property {string|null} [last_camera_name] - Name of the last active camera (for profile inheritance)
 * @property {boolean} eula_accepted - Whether the user has accepted the End User License Agreement
 * @property {string} indi_server_host - INDI server host
 * @property {number} indi_server_port - INDI server port
 * @property {boolean} focus_mode - Hold the cosmetic pipeline stages off while framing and focusing.
 */

/**
 * Capture status response
 * @typedef {object} CaptureStatus
 * @property {string} state
 * @property {number} frame_count
 * @property {number} stacked_count
 * @property {number} rejected_count
 * @property {string|null} last_error
 * @property {number|null} started_at
 * @property {number} exposure_us
 * @property {number} gain
 */

/**
 * Camera list entry
 * @typedef {object} Camera
 * @property {string} id
 * @property {string} name
 * @property {boolean} connected
 * @property {string|null} [provider]
 * @property {number|null} [index]
 * @property {CameraRole|null} [role] - Which position this camera occupies, or `None` if it is not connected.
 * @property {CameraPhase|null} [phase] - Lifecycle phase of a connected camera. The client's phase map is otherwise built from `camera_phase_changed` events alone, which a page opened later never saw.
 * @property {number|null} [warmup_remaining_s] - Seconds until a warm-up in progress is cut short, at the latest.
 * @property {CameraInfo} info
 */

/**
 * Camera info response
 * @typedef {object} CameraInfo
 * @property {string} id
 * @property {string} name
 * @property {number} max_width
 * @property {number} max_height
 * @property {number} pixel_size_x_um
 * @property {number} pixel_size_y_um
 * @property {string} sensor_type
 * @property {boolean} has_cooler
 * @property {boolean} has_dew_heater
 * @property {number|null} [min_temp_c]
 * @property {number|null} [max_temp_c]
 * @property {number} bit_depth
 * @property {number} min_exposure_us
 * @property {number} max_exposure_us
 * @property {number} min_gain
 * @property {number} max_gain
 * @property {Array<SensorMode>} [sensor_modes] - Sensor (dual sampling) modes reported by the camera. Empty when unsupported.
 */

/**
 * Simulated camera configuration response
 * @typedef {object} SimulatorConfig
 * @property {boolean} configured
 * @property {string|null} [directory]
 * @property {number|null} [file_count]
 * @property {number|null} [camera_count]
 * @property {boolean|null} [was_added]
 * @property {string|null} [message]
 */

/**
 * Push-To status response
 * @typedef {object} PushToStatus
 * @property {boolean} solver_ready - Whether the solver database is loaded
 * @property {boolean} is_solving - Whether a plate solve is currently in progress
 * @property {CatalogEntry|null} current_target - Current target (if set)
 * @property {Coordinate|null} last_position - Last solved position (if available)
 * @property {PushToDirection|null} direction - Push direction to target (if both position and target are set)
 */

/**
 * Catalog entry response
 * @typedef {object} CatalogEntry
 * @property {string} designation
 * @property {string|null} [name]
 * @property {string} catalog_type
 * @property {number} ra_degrees
 * @property {number} dec_degrees
 * @property {string} ra_string
 * @property {string} dec_string
 * @property {string} object_type
 * @property {number|null} [magnitude]
 * @property {string} constellation
 * @property {string|null} [messier] - Messier number ("M4") when the object is in the Messier catalog
 * @property {string|null} [matched_name] - The alias or identifier a search matched ("C 69"), when neither designation nor name did
 */

/**
 * ASTAP installation status response
 * @typedef {object} AstapStatus
 * @property {boolean} binary_installed - Whether ASTAP CLI binary is installed and executable
 * @property {string|null} [binary_path] - Path to the ASTAP binary (if installed)
 * @property {boolean} database_installed - Whether at least one star database is installed
 * @property {string|null} [database_path] - Path to the primary database directory (if installed)
 * @property {string|null} [database_type] - Primary installed database type (if any)
 * @property {Array<InstalledDatabaseInfo>} installed_databases - All installed databases with their paths
 * @property {boolean} ready - Whether the system is ready for plate solving
 */

/**
 * Available database types for installation
 * @typedef {object} DatabaseType
 * @property {string} id - Database identifier (D80, G05, W08)
 * @property {string} description - Human-readable description
 * @property {number} min_fov_deg - Minimum FOV in degrees this database supports
 * @property {number} max_fov_deg - Maximum FOV in degrees this database supports
 * @property {string} size - Approximate download size (e.g., "~3GB")
 * @property {boolean} installed - Whether this database is already installed
 */

/**
 * Catalog installation status response
 * @typedef {object} CatalogStatus
 * @property {boolean} installed - Whether the catalog is installed
 * @property {string|null} [catalog_path] - Path to the catalog directory (if installed)
 * @property {boolean} ngc_file_exists - Whether NGC.csv exists
 * @property {boolean} addendum_file_exists - Whether addendum.csv exists
 * @property {boolean} hyg_file_exists - Whether hyg_stars.csv exists
 * @property {number|null} [object_count] - Number of objects loaded (if catalog was parsed)
 */

/**
 * The "AI compute" setting: let the benchmark decide, or force one rung.
 * @typedef {'auto'|'npu'|'discrete_gpu'|'integrated_gpu'|'cpu'} AiComputePreference
 */

/**
 * Region of interest for alignment
 * @typedef {object} AlignmentRoi
 * @property {number} x - X coordinate of top-left corner
 * @property {number} y - Y coordinate of top-left corner
 * @property {number} width - Width of the ROI
 * @property {number} height - Height of the ROI
 */

/**
 * Algorithm used for background extraction
 * @typedef {'grid_bilinear'|'rbf'} BackgroundExtractionAlgorithm
 */

/**
 * Hardware-specific capture settings scoped to a single camera (keyed by `"{provider}/{model_name}"` in `CaptureSettings::camera_profiles`).
 * @typedef {object} CameraCaptureProfile
 * @property {number} exposure_us
 * @property {number} gain
 * @property {number} offset
 * @property {number} bin
 * @property {boolean} cooler_enabled
 * @property {number|null} target_temp_c
 * @property {DualSamplingMode|null} sensor_mode_override
 * @property {boolean} cooler_fast_mode
 * @property {boolean} dew_heater_enabled
 * @property {number} dew_heater_power
 */

/**
 * DTO for CameraPhase serialization (snake_case to match JS event handling).
 * @typedef {'disconnected'|'idle'|'precooling'|'capturing'|'guiding'|'warming_up'|'recovering'} CameraPhase
 */

/**
 * What a connected camera is for.
 * @typedef {'main'|'guide'} CameraRole
 */

/**
 * Coordinate response (simplified)
 * @typedef {object} Coordinate
 * @property {number} ra_degrees
 * @property {number} dec_degrees
 * @property {string} ra_string
 * @property {string} dec_string
 */

/**
 * Spatial denoising of the streamed image.
 * @typedef {object} DenoiseSettings
 * @property {boolean} enabled - The master switch, and a Pro control like the rest of this block. Off gives `DenoiseConfig::OFF`, which the encoders guarantee is *byte-identical* to the pre-denoise output rather than merely equivalent, so the toggle costs nothing.
 * @property {boolean} chroma - Guided-filter smoothing of the chroma planes, against luminance as the guide. Removes colour mottle; the eye resolves little chroma detail, so this is the cheap half with almost nothing to lose.
 * @property {number} chroma_strength - How far the chroma planes move toward the filtered result, `0..=1`.
 * @property {number} background_grain - How hard the render fights sky grain, `0..=1`. The "Background Grain" control.
 * @property {number} luma_strength - Scales the mid-scale wavelet thresholds (levels 2-4), `0..=1`. `1.0` is both the tuned value and the ceiling; `0.0` is the wavelet's genuine off switch.
 * @property {number} detail - How much the wavelet raises the target's own structure, `0..=1`. The "Detail" control; `0` renders exactly as before it existed.
 * @property {boolean} ai - The AI denoiser (Pro): a small network run after the stretch, taking over the finest four wavelet levels and Detail while it runs. Off by default. Needs the master switch on, like every filter here.
 * @property {AiComputePreference} ai_compute - The "AI compute" choice (Pro): which unit runs the network. Auto takes the hardware benchmark's pick; a forced rung this machine cannot use falls back to Auto. Unknown values read as Auto ([`AiComputePreference`]'s own `Deserialize`).
 */

/**
 * Dual sampling sensor mode (Player One terminology). Only meaningful for cameras that advertise sensor-mode selection — other providers ignore it.
 * @typedef {'normal'|'low_readout_noise'} DualSamplingMode
 */

/**
 * Settings specifically for the eyepiece view feature
 * @typedef {object} EyepieceSettings
 * @property {boolean} binoview - Enable Binoview
 * @property {number} screen_width - Screen width
 * @property {number} screen_height - Screen height
 * @property {string} screen_measurement - Measurement unit (e.g. "mm", "inches")
 * @property {number} screen_resolution_x - Screen resolution X
 * @property {number} screen_resolution_y - Screen resolution Y
 * @property {boolean} circular_view - Enable Circular view
 * @property {number} intensity - Dark background enhancement intensity (0.0 to 1.0)
 * @property {number} black_floor - Where black sits, as a signed fraction of full scale, in `[-0.09, 0.5]`. **Positive** lifts the output floor — an OLED shows a zero pixel fully off, and the autostretch black point clamps a few percent of sky pixels to zero, reading as black speckle at the eyepiece. **Negative** pushes the sky toward black instead, anchored to the sky rather than full scale: `-0.052` puts the floor at sky level whether the sky is nominal or bright, so one setting behaves the same on every target (sky otherwise sits at 14-17 output levels, a visible grey — dimming via the stretch would dim the target too; see [`intensity`](Self::intensity)).
 * @property {boolean} darker_sky - Let the darkening half of `black_floor` clip to true black instead of scaling the sky down (`render::output::sky_shadow`).
 * @property {boolean} dither - Blue-noise dithering at the 8-bit conversion, to keep smooth gradients from banding once denoising removes the noise that currently masks the steps.
 * @property {EyepieceStreamResolution} stream_resolution - The RGB8+LZ4 size every `/eyepiece_quality` client receives.
 */

/**
 * The subset of [`Resolution`] the eyepiece quality stream offers. No 1080p: the eyepiece screens this view exists for are at least 1440 px on their short edge, and a variant that cannot be stored cannot be selected by a stale client either — `serde` rejects it.
 * @typedef {'native'|'uhd2160'|'qhd1440'} EyepieceStreamResolution
 */

/**
 * Information about a single installed database
 * @typedef {object} InstalledDatabaseInfo
 * @property {string} id - Database identifier (D80, G05, W08)
 * @property {string} database_path - Path to this database's directory
 * @property {number} min_fov_deg - Minimum FOV in degrees this database supports
 * @property {number} max_fov_deg - Maximum FOV in degrees this database supports
 */

/**
 * Push-To direction response
 * @typedef {object} PushToDirection
 * @property {number} angle_deg - Angle to push in degrees, in the image frame: 0 = screen up, 90 = screen right, rotation is clockwise. The plate-solved camera rotation and parity are already applied, so this can be used directly as an SVG/CSS rotation for a chevron that points "up" at 0°. For a celestial-frame label use `direction_hint`.
 * @property {number} distance_deg - Angular distance to target in degrees
 * @property {boolean} is_close - Whether within fine-adjustment range (<1 degree)
 * @property {string} direction_hint - Direction hint (N, NE, E, SE, S, SW, W, NW, OK)
 * @property {string} direction_full - Full direction description
 * @property {Coordinate|null} current_position - Current position (if solved)
 * @property {Coordinate|null} target - Target position
 */

/**
 * Which capture modes write their raw frames to disk.
 * @typedef {object} RawFrameSaving
 * @property {boolean} live_view
 * @property {boolean} wanderer
 * @property {boolean} stacking
 * @property {boolean} guide
 */

/**
 * Rejection method for stacking.
 * @typedef {'None'|'SigmaClip'|'WinsorizedSigmaClip'|'MinMax'} RejectionMethod
 */

/**
 * A resolution the observer picks, as a bounding box: the frame is fitted inside it with its aspect ratio intact and never upscaled. Serves three settings:
 * @typedef {'uhd2160'|'qhd1440'|'hd1080'|'native'} Resolution
 */

/**
 * Corrections that run on the raw CFA mosaic, before demosaic.
 * @typedef {object} SensorCorrectionSettings
 * @property {number} hot_pixel_sigma - How far above its brightest same-colour neighbour a sample must sit to count as hot, in sigmas of that colour site's own noise.
 * @property {boolean} fpn_removal - Flatten per-row and per-column readout offsets.
 * @property {boolean} superpixel_debayer - Bin each 2x2 CFA quad to one RGB pixel instead of interpolating.
 */

/**
 * Camera sensor mode DTO (dual sampling mode slot)
 * @typedef {object} SensorMode
 * @property {number} index
 * @property {string} name
 * @property {string} [description]
 */

/**
 * Available stacking types
 * @typedef {'deep_sky'|'planetary'|'comet'} StackingType
 */

/**
 * @typedef {'low'|'medium'|'high'} StretchAggressiveness
 */

/**
 * Telescope and camera parameters for FOV calculation
 * @typedef {object} TelescopeSettings
 * @property {number|null} focal_length_mm - Telescope focal length in mm
 * @property {number|null} pixel_size_x_um - Pixel size X in micrometers (manual override or from camera database)
 * @property {number|null} pixel_size_y_um - Pixel size Y in micrometers (manual override or from camera database)
 * @property {number|null} sensor_width_px - Sensor width in pixels
 * @property {number|null} sensor_height_px - Sensor height in pixels
 * @property {number|null} barlow_coeff - Barlow/reducer coefficient (effective_fl = focal_length * coeff; default 1.0)
 */

/**
 * Weighting preset for quality-based frame weighting during stacking
 * @typedef {'disabled'|'balanced'|'galaxies'|'nebulae'|'fwhm_only'|'snr_only'} WeightingPreset
 */
