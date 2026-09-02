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

On the reference IMX533 frame this alone takes visible sky noise from 6.8 to 5.7
output levels, with integrated target brightness unchanged to within 0.2 %.
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

By default the finest scale of all is left untouched, because that is where star
cores sit. On the reference frame, with the colour filter also on, sky noise goes
from 6.8 to 4.7 output levels while integrated target brightness moves by 0.4 %
and the brightest star is unchanged.

**Structure strength** scales the thresholds for the larger scales — the soft
mottle across the target rather than the fine speckle. 100 % is the tuned
default. Lower it if the target starts looking soft or plastic; raising it leans
harder on faint nebulosity, so go carefully.

**Star protection** is the one that actually moves the background. It decides how
much of that finest scale is left alone — and since almost all the grain lives
there, it is the only control that visibly changes how smooth the sky looks. At
100 % nothing about the stars changes and the speckle stays; at 0 % sky noise on
the reference frame falls from 4.7 to 1.5 output levels, a 4.6x reduction, with
target brightness still within half a per cent.

The trade is real, so find it by eye: bring protection down until the tightest
stars start to soften, then back off a step. On a camera whose frames arrive at
close to your screen's resolution — an IMX464 on a 1440p display, say — this
control is doing nearly all the work, because there is no spare resolution for
the resize to average away first.

::: tip When to turn this one off
This is the setting that can destroy signal. If the target starts looking
smeared, waxy, or like a painting, turn Background Grain off before adjusting
anything else — the difference is much easier to judge by switching it on and
off than by nudging the strength.
:::

## Processing Resolution

Under **Settings → Preview**, and a different lever from the two filters above:
it decides the size everything upstream of them runs at — background extraction,
colour balance, the auto-stretch solve — rather than how much smoothing happens.

**Native** is the default and uses every sensor pixel. The lower settings box-
average the frame down first, which is a large speed-up on a small board and
removes noise on the way, at the cost of detail.

Two things worth knowing before you move it:

- **It re-grades the picture.** The auto-stretch is solved from the frame's own
  noise level, and averaging pixels together lowers that. Changing this setting
  visibly lifts or drops the shadows — around a 25 % change in shadow gain at 2×.
  That is not a bug to work around; it is why the setting is fixed for the whole
  session rather than following whoever happens to be connected.
- **It is all-or-nothing.** The reduction is by a whole factor of two, so a
  camera that does not have twice the pixels your chosen size needs will not bin
  at all and the setting will do nothing.

::: tip Which to pick
Leave it on **Native** unless the live view is not keeping up. If it is — a
Raspberry Pi with a large sensor is the usual case — drop it to the size you
actually view at and check the shadows still look right.
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
