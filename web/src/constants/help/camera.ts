/** Help for the camera's own settings: hardware, simulator, reconnects, focus image. */
export const CAMERA_HELP = {
    simulator_directory: 'The local path where the simulator looks for source images (FITS, TIFF, or PNG).',
    simulated_camera: 'Uses images from a directory instead of a real camera.',
    simulated_preload_count:
        'Number of images to keep in memory for the simulator. Higher values make playback smoother but use more RAM.',
    bin: 'Combines adjacent pixels to increase sensitivity and Signal-to-Noise Ratio (SNR) at the cost of resolution. 2x2 binning is 4x more sensitive. Changing binning will automatically reset the current stack.',
    cooler_enabled:
        "Activates the camera's TEC cooler during capture to lower sensor temperature and reduce thermal noise. Cooling only applies while a capture session is running.",
    target_temp_c:
        'Target sensor temperature in Celsius. Pick a value warm enough that your TEC can hold it consistently (typically 20-30°C below ambient).',
    cooler_fast_mode:
        'Skips the rate-limited temperature ramp (5°C/min) — the camera cools and warms up as fast as the hardware allows. Not recommended for long-term use: abrupt temperature swings can stress the sensor and risk condensation on the cover glass.',
    dew_heater_enabled:
        'Activates the anti-dew heater at the front of the camera sensor to prevent condensation on the optical window.',
    dew_heater_power:
        'Controls the power level of the anti-dew heater.',
    daylight_balance:
        "How this camera's colours look in sunlight, which lights every planet, the Moon and the Sun. Read off the Moon by itself: whenever Planetary shows one, at any exposure and with or without auto-stretch, its colour is measured every two seconds and the balance that gives the Moon its real, faintly warm grey is remembered for this camera. Every other target keeps its own colour through the same balance — Mars stays orange, an H-alpha Sun red. Until the Moon has been seen, a balance typical of colour cameras is used. Used by Planetary only; saved FITS and SER files keep the sensor's colour.",
    show_focus_image:
        'Displays a focusing image instead of "Waiting for frames" on the eyepiece endpoints to help with screen setup and focus adjustments.',
    force_focus_image_now:
        'Forces the focusing image to be displayed even when the camera stream is active, allowing you to check eyepiece focus at any time.',
    auto_reconnect:
        'Reopens the camera by itself after it drops out mid-session — a USB stall or a knocked cable. Retries a few times with a growing gap, then stops and tells you.',
    auto_resume_capture:
        'After the camera comes back, picks the capture up where it stopped: same mode, same settings, and the frames already stacked are kept.',
}
