# Noise Reduction

These two filters smooth the image you are looking at. They are the last thing
that touches a frame before it reaches your screen, and unlike the sensor
corrections they are a matter of taste — every denoiser has a setting at which
nebulae start looking like plastic, and only your eye at the eyepiece can find
where that is.

Find them under **Settings → Noise Reduction**.

## They run at the size you view, not the size you capture

Both filters work on the streamed image after it has been reduced to your
screen's resolution, not on the full sensor frame. On a 9-megapixel camera
feeding a 1440×1440 eyepiece display that is four and a half times less work for
exactly the same visible result — three quarters of a denoised sensor frame is
thrown away by the resize anyway.

There is a bonus in the ordering: that resize is itself an area average, which
already halves the noise before either filter starts.

## Colour Mottle

The blotchy colour patches in an otherwise grey background. They come from the
colour interpolation, which has to guess two of every pixel's three colours from
its neighbours, and guesses badly when the neighbours are noisy.

This is the cheap, safe half. Your eye resolves far less colour detail than
brightness detail, so colour can be smoothed hard with almost nothing to lose.
The filter is *guided* by the brightness image, which means it stops smoothing
wherever the brightness has an edge — a star keeps its own colour instead of
bleeding it across the sky beside it.

On the reference IMX533 frame this alone takes visible sky noise from 7.4 to 5.9
output levels, with integrated nebula brightness unchanged to within 0.2 %.
Leave it on.

**Colour strength** controls how far the colour planes move toward the smoothed
result. Lower it if faint colour in the target starts washing out.

## Background Grain

The luminance grain — the fine speckle across the whole background. This filter
separates the image into scales and smooths each one by a different amount:
hardest just above the size of a star, backing off as the structures get larger.

The order matters and is the opposite of what seems obvious. Denoising hardest
at the *coarse* scales would remove the most visible mottle, but coarse scales
are also where faint nebulosity lives — the Dumbbell's outer lobes are coarse
structure, and a filter tuned that way erases them along with the noise.

The finest scale of all is left untouched, because that is where star cores sit.
On the reference frame, with the colour filter also on, sky noise goes from 7.4
to 4.5 output levels — a third less grain — while integrated nebula brightness
moves by 0.4 % and the brightest star is unchanged.

**Grain strength** scales every threshold. 100 % is the tuned default. Lower it
if the target starts looking soft or plastic; raise it only if the background
still looks grainy and the target does not.

::: tip When to turn this one off
This is the setting that can destroy signal. If the target starts looking
smeared, waxy, or like a painting, turn Background Grain off before adjusting
anything else — the difference is much easier to judge by switching it on and
off than by nudging the strength.
:::

## Not applied to planetary targets

Both filters are skipped automatically for **Planetary** stacking. Lucky imaging
exists to recover exactly the fine detail a denoiser removes, so smoothing the
result would undo the whole point of the mode.

## What this does not fix

Coloured dots in the background are hot pixels, not noise — they are in the same
place in every frame, and no amount of smoothing removes a defect that does not
average away. Turn on **Hot Pixel Rejection** under
[Sensor Corrections](/sensor-corrections) instead.

Similarly, soft banding across the frame is a readout pattern; it needs
**Row/Column Pattern Removal**, and smoothing it only turns sharp bands into
soft streaks, which is worse.
