/** Help for the capture panel, the camera list and the status bar. */
export const CAPTURE_HELP = {
    cameras:
        'Choose the camera to configure. One imaging camera and one guide camera can be connected at once — use the arrow next to Connect to attach a guide camera.',
    precooling: 'Sensor is still cooling — capture will start with elevated dark current.',
    frame_counts:
        'Total: Frames captured and processed. Rejected: Captured frames that failed alignment or quality checks. Dropped: Frames discarded because the CPU was too busy to process them.',
    focus_mode:
        'Trades image quality for frame rate while you focus or hunt for a target. Holds off the six stages that cost time per frame and buy nothing at a focus mask: background subtraction, shadow saturation boost, row/column pattern removal, colour mottle, structure strength and dithering. Hot pixel rejection keeps running - Push-To cannot plate-solve a frame full of them. Their Settings controls are held while the mode is on and every one goes back to your value when you switch it off. Background Grain is not one of them: it also sets how the stretch trades brightness against grain, which this mode has no business changing.',
    focus_mode_held: 'Greyed settings are held off by Focus/Finder mode.',
    focus_mode_while_stacking:
        'Not available while you are stacking. Focus/Finder mode turns off row/column pattern removal, which runs before the frames reach the stack - and that banding sits in the same place in every frame, so once it is averaged in nothing can take it out again. Stop the capture to focus, or switch to Live view.',
    auto_stretch:
        'Automatically transforms the image to make faint details and colors visible - stretching',
    stretch_aggressiveness:
        'How hard the dark end of the image is boosted, and with it how bright the background sits.\n• Star Fields: the gentlest. Darker, cleaner background, at the cost of some faint outer structure.\n• Deep Sky: the default, for nebulae, galaxies and clusters alike.\nBoth render stars the same way — the difference is what happens below them.',
    stacking:
        'Determines how the camera feed is processed:\n• Live view: Raw feed, no stacking. Best for focusing.\n• Wanderer: Auto-stacks when stationary, resets on movement.\n• Stacking: Continuous real-time image accumulation.',
    stacking_type:
        "Specifies the processing pipeline:\n• Deep Sky: Optimized for long exposures of nebulae and galaxies.\n• Planetary: High-speed imaging for planets/Moon.\n• Comet: Special alignment that tracks the comet's motion",
    guide_capture_mode:
        'The guide camera always runs as a live view. Nothing it captures is stacked, so Wanderer and Stacking are not offered for it — select the imaging camera to change those.',
    exposure:
        'Controls how long the camera sensor collects light for each frame. Longer exposures reveal fainter details but are more sensitive to tracking errors.',
    gain: 'Electronic amplification of the signal. Higher gain increases sensitivity but also introduces more read noise.',
    wanderer_mode:
        'Automatically resets the stack when you move to a new target. Once stationary, stacking restarts and the live stack is displayed.',
    comet_pro: 'Comet centroid tracking and aggressive star rejection are available in Night Amplifier Pro.',
}
