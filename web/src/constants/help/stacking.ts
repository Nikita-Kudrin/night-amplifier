/** Help for stacking and storage settings. */
export const STACKING_HELP = {
    planetary_auto_tracking: "Automatically track the planet's centroid in the ROI to prevent drift.",
    planetary_multi_point_alignment:
        'Use Inverse Distance Weighting on multiple surface points to combat seeing distortion.',
    weighting_preset:
        'Determines how much each frame contributes based on quality:\n• Disabled: All frames weighted equally.\n• Balanced: General purpose blending.\n• Galaxies: Favors sharpest frames (FWHM).\n• Nebulae: Favors highest signal (SNR).\n• FWHM Only: Ignores SNR, prioritizes sharpness.\n• SNR Only: Ignores sharpness, prioritizes signal.\nChanges apply immediately to subsequent frames.',
    rejection_method:
        'Outlier rejection algorithm to remove satellites, planes, or hot pixels from the stack:\n• None: Simple average. Fast and clean for noise-free data.\n• Sigma Clipping: Statistically rejects values too far from the mean. Great for satellite trails.\n• Winsorized: Clips extreme values to the rejection threshold rather than discarding. More stable for smaller stacks.\n• Min-Max: Discards the highest and lowest pixel values. Simple and effective for sparse satellite trails.\nChanges apply immediately to subsequent frames.',
    rejection_sigma:
        'Controls the sensitivity of the rejection algorithm. Lower values (e.g., 2.0) are more aggressive; higher values (e.g., 4.0) preserve more signal. Changes apply immediately to subsequent frames.',
    auto_crop:
        "Crops the stack to the area nearly every frame covers, cutting away the thin, noisy border that drift, field rotation or a bumped mount leave. Applies to the live view, both eyepiece views and the saved FITS and PNG (the FITS records where the crop starts as XORGSUBF/YORGSUBF). It starts once 20 frames are stacked, so one bumped frame is outvoted, and shrinks as the field drifts, in occasional steps rather than every frame. Deep Sky only.",
    raw_frame_saving:
        "Saves captured frames to 'captures/raw/', chosen per capture mode. Deep Sky and Comet write one FITS file per exposure; Planetary writes a single SER container per session. Each session gets its own folder, tagged with the mode that filled it: '-live', '-wanderer', '-stacking' or '-guide'. In debug mode (server started with debug logging) the guide camera's frames can be saved too, alongside the imaging one, into a folder of its own; outside it the guide switch is hidden and its frames are never saved.\nLive view writes a frame per exposure, so at short focusing exposures it can fill a card quickly and outrun a slow disk - frames are dropped rather than delaying capture.",
    save_stacked_image:
        "Saves the final stack as a FITS and a stretched PNG to 'captures/stacked/'. Stacking mode only - Live view builds no stack, and Wanderer discards its stack whenever the telescope moves.",
}
