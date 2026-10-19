/** Help for the eyepiece settings. */
export const EYEPIECE_HELP = {
    eyepiece_binoview:
        'Splits the screen into two independent copies of the image based on physical screen dimensions and resolution.',
    eyepiece_screen_settings:
        'Configure physical screen dimensions and resolution to calculate accurate split for Binoview.',
    eyepiece_stream_resolution:
        'Image size for the lossless eyepiece view (/eyepiece_quality). Native, the default, sends every processed pixel up to 3840 px a side: the best quality. Lossless frames are large, so choose a smaller size, such as 1440p for a 1440 × 1440 panel, to save bandwidth and get a more responsive stream on weak Wi-Fi or with several eyepieces. Changes apply from the next frame.',
    eyepiece_circular_view:
        'Crops the view to a circle, emulating the experience of looking through a real telescope eyepiece.',
    eyepiece_intensity:
        'Darkens the background sky for OLED screens and pulls up the contrast of objects. Higher values also push more of the sky\'s noise below black, so the background looks smoother.',
    eyepiece_black_floor:
        'Sets where black sits.\n• Positive keeps the darkest pixels just above pure black. An OLED switches a black pixel fully off, so sky pixels the stretch clipped to zero read as black speckle at eyepiece magnification rather than as sky. Raise it if the background shows hard black dots.\n• Negative pushes the sky itself toward black without dimming the target, which is what lowering Black level does instead. Around -4.5% puts the floor at the sky level; the far end takes the background down by about 60%, or by around 80% with Darker sky on. The negative half needs a measured sky level, so it does nothing with Auto stretch off or in Planetary mode.',
    eyepiece_darker_sky:
        'Lets the darkening half of Black floor reach true black instead of rolling off into it. Buys the deepest possible sky and a little more separation between the target and the background, and costs the black speckle the positive half of Black floor exists to remove - around a third of the sky ends up fully off. No effect while Black floor is positive.',
    eyepiece_dither:
        'Adds a fine random-looking pattern, below one brightness step, before the image is reduced to 8 bits, so smooth gradients do not band. With a noisy sky the noise already does this and you will see no difference; it matters once the background is smooth. It arrives whole only on /eyepiece_quality; the compressed views keep about half of it.',
}
