/** Help for image processing and noise reduction settings. */
export const IMAGE_QUALITY_HELP = {
    background_subtraction:
        'Removes gradients caused by light pollution or moonlight, resulting in a more even background across the image.',
    background_extraction_algorithm:
        'Choose the background removal algorithm:\n• Grid (Fast): Grid-based interpolation. Fast and stable for general EAA.\n• RBF (High Quality): Radial Basis Function. Intelligently routes around nebulae to protect faint signals.',
    saturation_boost:
        "Intensifies colors in the darkest parts of the image, helping faint nebulae and galaxy arms 'pop' without over-saturating stars.",
    saturation_boost_strength:
        'Adjusts the intensity of the Shadow Saturation Boost. Higher values create more vibrant colors in faint areas.',
    auto_stretch_intensity:
        'Intensifies the colors of the image while perfectly preserving the luminance/black level.',
    hot_pixel_sigma:
        'Hot pixels - the coloured dots scattered through the background - are always replaced on the raw sensor mosaic. They sit in the same place in every frame, so stacking never removes them, and Push-To cannot plate-solve a frame full of them. This sets how far above its brightest neighbour a pixel must sit to count as hot. Lower catches more dots but risks the faintest stars; raise it if stars look softer or the count drops.',
    fpn_removal:
        'Flattens the small brightness offset every sensor row and column reads out with. Like hot pixels it does not average away with more frames, and on a drifting mount it shows up as soft banding. Not applied to planetary targets, where a bright disc fills enough of each line to move its level.',
    superpixel_debayer:
        'Turns each 2x2 sensor square into one colour pixel instead of interpolating. Halves the width and height, invents no colour noise, and keeps any surviving hot pixel to a single dot. Free on a large sensor that already oversamples your screen; a real loss of detail on a smaller one.',
    preview_resolution:
        'Resolution the whole preview pipeline runs at. Native uses every sensor pixel. The lower settings box-average the frame down first, which is much faster on a small board and removes noise on the way, but you lose detail and the picture will re-grade when you change it. Fixed for the session — it deliberately does not follow whoever is connected.',
    streaming_resolution:
        'Image size sent to every screen viewing the live view and the eyepiece view. All of them get the same picture, so larger sizes cost bandwidth and server time for every viewer. It never goes above the Processing Resolution. On a screen smaller than the image the browser shrinks it, which looks grainier than choosing the smaller size here. Changes apply from the next frame.',
    denoise_enabled:
        'Turns every noise filter on or off at once. Off shows the stacked image exactly as it comes out of the pipeline — useful for judging what the filters are doing to a target, and the quickest way back if something looks plastic. Background Grain switches off with the filters, including the brightness it trades: off, the picture is rendered as at the dial\'s default. Focus/Finder mode never touches this switch.',
    denoise_chroma:
        'Removes the blotchy colour patches in the background without touching brightness detail. The eye resolves far less colour detail than brightness, so this can smooth hard with almost nothing to lose. Cheap and safe — leave it on.',
    denoise_chroma_strength:
        'How far the colour planes move toward the smoothed result. Lower it if faint colour in the target starts washing out.',
    denoise_background_grain:
        'How hard to fight grain in the background. The two halves do different things, because the mechanisms available cost very different amounts.\n• Above 50% the picture gets smoother almost for free. This is the half that works on the coarse, blotchy grain you actually notice — measured across six sessions it takes it down 9-27% while the target dims by at most 1%. Start here.\n• 50% is the tuned default: the cheap mechanism fully spent, the expensive one no further than its own default.\n• Below 50% you are buying brightness back with grain. It returns the finest speckle first, which is nearly invisible at normal viewing size, and then starts weakening the tone curve, which is what actually brightens faint nebulosity and galaxy arms — on a deep session the target reads about 17% brighter at 0% than at 50%.\nIf you are chasing smooth background, go up. If you are chasing faint detail, go down. Judge it at the eyepiece on the target you are actually looking at.',
    denoise_luma_strength:
        'Scales the thresholds for the mid scales — the soft mottle across the target, not the fine speckle and not the broad background blotches the Background Grain dial handles. 100% is the tuned value and the maximum: past it the mottle does not go away, it moves out to a coarser scale, and stars start to show a ring. Lower it if the target looks soft or plastic; 0% turns the brightness denoiser off altogether, which is the setting to reach for if nebulae start looking like plastic.',
    denoise_detail:
        'Brings out the structure inside the target - the lanes, knots and edges of a nebula or a galaxy\'s arms - without touching the background or the stars. It raises contrast at the scales between fine speckle and broad glow, and only where the target stands above the sky, so the background grain does not grow with it. Stars are left alone, including stars sitting on the nebula, which is where ordinary sharpening leaves dark rings. The grain on the target sits at the same scales, so it rises too: if a short stack or a faint galaxy looks grainier, lower this. 0% shows the target as it was before this control existed. Towards 100% a bright star on a bright galaxy disk can pick up a faint dark ring. It works through the brightness denoiser, so it does nothing while Structure strength is at 0%.',
    denoise_ai:
        'Runs a small neural network over the stretched image to clean up the fine, pixel-scale grain. While it runs it replaces Structure strength and Detail; Colour Mottle and Background Grain keep working, and Background Grain above 50% also smooths the larger mottle the network cannot reach. It keeps a nebula\'s texture where the standard filters smooth it, but it removes less grain than they do and softens stars a little more: a different look, not a stronger filter. Needs Denoise on, and is held off in Focus/Finder mode and on the guide camera. It is the most expensive stage of a render, so expect a slower refresh on a small board. Model: astro_denoiser_N2N_N2S "train_13" by Andriy Melnykov (MIT licence).',
    ai_compute:
        'Which processor runs AI denoising. On first start Night Amplifier measures every NPU and GPU it can use on four IMX533 frames, once per computer, and Auto takes the fastest one that is not the CPU - during a session the CPU is busy capturing and stacking. Pick a rung to force it, for example the integrated GPU to spare a laptop battery. The chosen processor is only woken while AI denoising is on, and released a few minutes after it goes off. Options this computer lacks or cannot use are greyed out with the reason; missing drivers and runtimes are listed under System dependencies in the manual.',
    ai_compute_remeasure:
        'Runs the hardware benchmark again. Use it after installing a driver or runtime, or to give a processor recorded as crashing Night Amplifier another try - if it crashes again, it is skipped again at the next start. Not available while a capture is running.',
}
