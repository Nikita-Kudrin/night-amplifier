/**
 * Application constants and configuration values
 */

// Exposure presets by unit
export const EXPOSURE_PRESETS = {
    us: [
        {label: '100', us: 100},
        {label: '500', us: 500},
        {label: '1000', us: 1000},
        {label: '2000', us: 2000},
        {label: '5000', us: 5000},
    ],
    ms: [
        {label: '5', us: 5000},
        {label: '10', us: 10000},
        {label: '20', us: 20000},
        {label: '40', us: 40000},
        {label: '50', us: 50000},
        {label: '100', us: 100000},
        {label: '200', us: 200000},
        {label: '300', us: 300000},
        {label: '400', us: 400000},
    ],
    s: [
        {label: '0.5', us: 500000},
        {label: '1', us: 1000000},
        {label: '2', us: 2000000},
        {label: '3', us: 3000000},
        {label: '5', us: 5000000},
        {label: '10', us: 10000000},
        {label: '15', us: 15000000},
        {label: '30', us: 30000000},
        {label: '60', us: 60000000},
    ],
}

// Binning options
export const BINNING_OPTIONS = [
    {value: 1, label: '1x1'},
    {value: 2, label: '2x2'},
    {value: 3, label: '3x3'},
    {value: 4, label: '4x4'},
]

// Gain limits
export const GAIN_LIMITS = {
    min: 0,
    max: 500,
    default: 0,
}

// Shadow saturation boost strength limits
export const SATURATION_BOOST_LIMITS = {
    min: 0.1,
    max: 1.0,
    step: 0.05,
    default: 0.5,
}

// Simulated camera preload limits
export const SIMULATED_PRELOAD_LIMITS = {
    min: 1,
    max: 20,
    step: 1,
    default: 5,
}

// Stretch aggressiveness options
export const STRETCH_AGGRESSIVENESS_OPTIONS = [
    {value: 'high', label: 'High (Nebulae)'},
    {value: 'medium', label: 'Medium'},
    {value: 'low', label: 'Low (Star Fields)'},
]

// Frame weighting preset options for quality-based stacking
export const WEIGHTING_PRESET_OPTIONS = [
    {value: 'disabled', label: 'Disabled'},
    {value: 'balanced', label: 'Balanced'},
    {value: 'galaxies', label: 'Galaxies'},
    {value: 'nebulae', label: 'Nebulae'},
    {value: 'fwhm_only', label: 'FWHM Only'},
    {value: 'snr_only', label: 'SNR Only'},
]

// Outlier rejection method options for stacking
export const REJECTION_METHOD_OPTIONS = [
    {value: 'None', label: 'None (Average)'},
    {value: 'SigmaClip', label: 'Sigma Clipping', pro: true},
    {value: 'WinsorizedSigmaClip', label: 'Winsorized', pro: true},
    {value: 'MinMax', label: 'Min-Max', pro: true},
]

// Background extraction algorithm options
export const BACKGROUND_ALGORITHM_OPTIONS = [
    {value: 'grid_bilinear', label: 'Grid (Fast)'},
    {value: 'rbf', label: 'RBF (High Quality)', pro: true},
]

// Telescope setup limits
export const TELESCOPE_LIMITS = {
    focal_length_min: 50,
    focal_length_max: 10000,
    barlow_min: 0.1,
    barlow_max: 5.0,
    barlow_step: 0.05,
    barlow_default: 1.0,
    pixel_size_min: 0.5,
    pixel_size_max: 20.0,
    pixel_size_step: 0.01,
}

// Capture states
export const CAPTURE_STATES = {
    IDLE: 'Idle',
    STARTING: 'Starting',
    CAPTURING: 'Capturing',
    STOPPING: 'Stopping',
    ERROR: 'Error',
}

// WebSocket reconnection settings
export const WS_RECONNECT = {
    interval: 3000,
    maxAttempts: 10,
    pingInterval: 30000,
}

// Zoom limits for LiveView
export const ZOOM_LIMITS = {
    min: 0.5,
    max: 5,
    zoomInFactor: 1.2,
    zoomOutFactor: 0.8,
    wheelZoomIn: 1.1,
    wheelZoomOut: 0.9,
}

// RGB8+LZ4 stream magic number
export const RGB8_MAGIC = 0x53413038

// Cooler temperature defaults (used as a fallback when the camera does not advertise its range)
export const COOLER_TEMP_LIMITS = {
    min: -40,
    max: 20,
    step: 1,
    default: -10,
}

// Default settings values
export const DEFAULT_SETTINGS = {
    exposure_us: 1000000,
    gain: 0,
    offset: 10,
    bin: 1,
    auto_stretch: true,
    stacking: true,
    background_subtraction: true,
    background_extraction_algorithm: 'grid_bilinear',
    save_raw_frames: false,
    save_stacked_image: false,
    weighting_preset: 'balanced',
    rejection_method: 'None',
    rejection_sigma: 2.5,
    stretch_aggressiveness: 'medium',
    saturation_boost: false,
    saturation_boost_strength: 0.5,
    simulated_camera: false,
    simulated_preload_images: 5,
    wanderer_mode: false,
    cooler_enabled: false,
    target_temp_c: null,
    cooler_fast_mode: false,
    eyepiece: {
        binoview: true,
        screen_width: 140.0,
        screen_height: 67.0,
        screen_measurement: 'mm',
        screen_resolution_x: 2880,
        screen_resolution_y: 1440,
    },
    dew_heater_enabled: true,
    dew_heater_power: 10,
    telescope: {
        focal_length_mm: null,
        pixel_size_x_um: null,
        pixel_size_y_um: null,
        sensor_width_px: null,
        sensor_height_px: null,
        barlow_coeff: null,
    },
}

// Help tooltips for UI elements
export const HELP_TEXTS = {
    bin: 'Combines adjacent pixels to increase sensitivity and Signal-to-Noise Ratio (SNR) at the cost of resolution. 2x2 binning is 4x more sensitive. Changing binning will automatically reset the current stack.',
    auto_stretch:
        'Automatically applies transformation to make faint details visible to the human eye.',
    stretch_aggressiveness:
        'Controls how strongly the dark areas are boosted. High is best for extremely faint nebulae, Low preserves star colors and contrast.',
    background_subtraction:
        'Removes gradients caused by light pollution or moonlight, resulting in a more even background across the image.',
    background_extraction_algorithm:
        'Choose the background removal algorithm:\n• Grid (Fast): Grid-based interpolation. Fast and stable for general EAA.\n• RBF (High Quality): Radial Basis Function. Intelligently routes around nebulae to protect faint signals.',
    saturation_boost:
        "Intensifies colors in the darkest parts of the image, helping faint nebulae and galaxy arms 'pop' without over-saturating stars.",
    saturation_boost_strength:
        'Adjusts the intensity of the Shadow Saturation Boost. Higher values create more vibrant colors in faint areas.',
    save_raw_frames:
        "Saves individual captured frames as FITS files to 'captures/raw/'. Only works in Stacking mode.",
    save_stacked_image:
        "Saves the final stack as a FITS and a stretched PNG to 'captures/stacked/'. Only works in Stacking mode.",
    weighting_preset:
        'Determines how much each frame contributes based on quality:\n• Disabled: All frames weighted equally.\n• Balanced: General purpose blending.\n• Galaxies: Favors sharpest frames (FWHM).\n• Nebulae: Favors highest signal (SNR).\n• FWHM Only: Ignores SNR, prioritizes sharpness.\n• SNR Only: Ignores sharpness, prioritizes signal.\nChanges apply immediately to subsequent frames.',
    simulated_camera: 'Uses images from a directory instead of a real camera.',
    simulated_preload_count:
        'Number of images to keep in memory for the simulator. Higher values make playback smoother but use more RAM.',
    stacking:
        'Determines how the camera feed is processed:\n• Live view: Raw feed, no stacking. Best for focusing.\n• Wanderer: Auto-stacks when stationary, resets on movement.\n• Stacking: Continuous real-time image accumulation.',
    stacking_type:
        "Specifies the processing pipeline:\n• Deep Sky: Optimized for long exposures of nebulae and galaxies.\n• Planetary: High-speed imaging for planets/Moon.\n• Comet: Special alignment that tracks the comet's motion",
    exposure:
        'Controls how long the camera sensor collects light for each frame. Longer exposures reveal fainter details but are more sensitive to tracking errors.',
    gain: 'Electronic amplification of the signal. Higher gain increases sensitivity but also introduces more read noise.',
    wanderer_mode:
        'Automatically resets the stack when you move to a new target. Once stationary, stacking restarts and the live stack is displayed.',
    eyepiece_binoview:
        'Splits the screen into two independent copies of the image based on physical screen dimensions and resolution.',
    eyepiece_screen_settings:
        'Configure physical screen dimensions and resolution to calculate accurate split for Binoview.',
    rejection_method:
        'Outlier rejection algorithm to remove satellites, planes, or hot pixels from the stack:\n• None: Simple average. Fast and clean for noise-free data.\n• Sigma Clipping: Statistically rejects values too far from the mean. Great for satellite trails.\n• Winsorized: Clips extreme values to the rejection threshold rather than discarding. More stable for smaller stacks.\nChanges apply immediately to subsequent frames.',
    rejection_sigma:
        'Controls the sensitivity of the rejection algorithm. Lower values (e.g., 2.0) are more aggressive; higher values (e.g., 4.0) preserve more signal. Changes apply immediately to subsequent frames.',
    telescope_focal_length: 'The focal length of your telescope in millimeters.',
    telescope_camera_sensor:
        'Select your camera from the database or enter pixel size manually. You can also auto-fill from the connected camera.',
    telescope_barlow:
        'Barlow/reducer coefficient. 1.0 = no barlow/reducer. Values > 1.0 for barlows (e.g. 2.0 for 2x barlow). Values < 1.0 for reducers (e.g. 0.63 for a 0.63x reducer).',
    telescope_fov:
        'Calculated Field of View based on your telescope and camera parameters. Sent to the plate solver for faster solving.',
    cooler_enabled:
        "Activates the camera's TEC cooler during capture to lower sensor temperature and reduce thermal noise. Cooling only applies while a capture session is running.",
    target_temp_c:
        'Target sensor temperature in Celsius. Pick a value warm enough that your TEC can hold it consistently (typically 20-30°C below ambient).',
    cooler_fast_mode:
        'Skips the rate-limited temperature ramp (5°C/min) — the camera cools and warms up as fast as the hardware allows. Not recommended for long-term use: abrupt temperature swings can stress the sensor and risk condensation on the cover glass.',
    dew_heater_enabled:
        'Activates the anti-dew heater at the front of the camera sensor to prevent condensation on the optical window.',
    dew_heater_power:
        'Controls the power level of the anti-dew heater. Typically 10-30% is sufficient for most conditions.',
}
