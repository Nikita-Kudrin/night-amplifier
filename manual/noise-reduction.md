# Noise Reduction

These two filters smooth the image you are looking at. They are the last thing
that touches a frame before it reaches your screen, and unlike the sensor
corrections they are a matter of taste — every denoiser has a setting at which
nebulae start looking like plastic, and only your eye at the eyepiece can find
where that is.

Find them under **Settings → Noise Reduction**. Noise reduction is a **Night Amplifier
Pro** feature: without Pro the section is shown locked, and the picture is the stacked
image exactly as the pipeline produces it — the tone curve is the same one Pro uses at
the default Background Grain setting, so the whole difference is the filters.

**Denoise** at the top of the section turns every filter on or off at once. Off shows the
stacked image untouched — exactly what the Community edition renders — which is the quickest
way to judge what the filters are doing to a target, and the way back if something starts to
look plastic. Background Grain greys out with the filters, and its brightness trade goes with
it: off, the tone curve is back at the dial's middle, so a target you brightened with a low
dial will look a little dimmer. At the default dial position nothing but the filters changes.

Colour Mottle and Structure strength are among the six that
[Focus/Finder mode](/getting-started#focus-finder-mode) holds off while you focus; their
controls grey out while it is on and come back to your values when you turn it off. The
Denoise switch is yours alone — Focus/Finder mode never touches it.

## The edges of the stack are denoised a little harder

When the mount drifts during a session, the edges of the frame are covered by fewer subs
than the middle, so they carry more noise. The filters know how much of the stack each part
of the frame holds and smooth the thinly covered parts correspondingly harder, so a stack's
border does not look grainier than its centre. Where every sub covered the frame the picture
is exactly what it would be otherwise — outlier rejection throwing a satellite trail or a
cosmic ray away does not count as a thin edge. A saved stacked PNG is smoothed the same way
as the live view.

This evens out *grain*, the fine speckle the filters can reach. It cannot remove a visible
line where one sub's edge ends — that is a real step in the image, not noise; the fix for it is
dithering or letting the stack grow past it.

## They run at the size you stream, not the size you capture

Both filters work on the streamed image after it has been reduced to its
[streaming resolution](#streaming-resolution), not on the full sensor frame. On a
9-megapixel camera streamed at 1440p to a 1440×1440 eyepiece display that is four
and a half times less work for exactly the same visible result — three quarters of
a denoised sensor frame is thrown away by the resize anyway.

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

What counts as an edge is measured against the noise of the frame in front of it,
so the filter behaves the same on a single sub and on a hundred-frame stack. It
used to use a fixed threshold, which on a deep stack was far above the frame's own
noise: stars stopped registering as edges and their colour spread into soft
patches across the background — the blotches you may remember on a globular
cluster. If you ever see those again, that threshold is the first place to look.

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

Each scale is measured against its own noise, not against the finest one's. That
matters because a stack's noise is not white — aligning and combining frames
correlates neighbouring pixels — so the broad mottle carries far more noise than
the fine speckle would predict. Thresholds derived from the finest scale were too
small to reach it.

**Background Grain** is one dial over three mechanisms, and what separates them is not
just what they cost but *what size of grain they can reach*. The speckle you notice in a
background is mostly broad and blotchy — tens of pixels across — not the fine pepper at
the pixel level.

**Above 50 % is the half that works on what you notice.** It brings in two extra, coarser
stages of the filter, the only ones that reach that broad mottle at all. They are close to
free: measured across six sessions they take it down between 9 % and 27 % while the target
dims by at most 1 %. If the background looks blotchy, this is the direction to go.

**Below 50 % you are buying brightness back**, and in two stages. From 50 % down to 25 %
it eases off the tone curve, which is what actually brightens faint nebulosity and galaxy
arms. Below 25 % the curve is already all the way back and the dial only returns the
finest speckle, which is nearly invisible at normal viewing size — so the bottom quarter
costs no brightness at all. On a deep session the target reads about 17 % brighter at 0 %
than at 50 %, and essentially all of that is won between 50 % and 25 %.

**50 %** is the tuned default: the point where the cheap mechanism is fully spent and the
expensive one has not been asked for anything beyond its own default.

The dial deliberately stops short of the tone curve's full range. Pushing that lever all
the way took 40 % of the target's brightness to buy 38 % of the grain — a straight
one-for-one trade, and not one worth offering. The coarse stages replaced that end of the
dial and buy the same smoothness about fifteen times more cheaply.

Two things worth knowing. The dial does not reach a tone curve share of zero: there, the
sky gets *grainier* the longer you integrate — the filter removes a fixed fraction of the
noise rather than holding the sky at a level, so nothing stops a deep stack's remaining
noise from showing. Measured over 106 frames, signal-to-noise peaked around 64 frames and
then fell back. The bottom of the dial stops just above that, where displayed grain is
flat with session length.

And on a camera whose frames arrive close to their streaming resolution — an IMX464
streamed at 1440p, say — the dial is doing nearly all the work, because there is no spare
resolution for the resize to average away first.

**Star Fields mode filters differently.** It is looking for points of light against an
empty sky and has no nebulosity to protect, so it leans harder on the finest scale — where
a star competes with single-pixel noise — and then sharpens the scales a star actually
occupies to put its peak back. On a wide field this roughly doubles the number of stars
you can see, and on a long focal length it trades a few of the very faintest for a
noticeably cleaner background.

**Structure strength** scales the thresholds for the mid scales — the soft mottle across
the target, not the fine speckle and not the broad background blotches the dial above
handles. **100 % is both the tuned value and the maximum.** Past it the filter stops
helping and starts hurting: the mottle is not removed, it is pushed out to a scale the
filter cannot reach, and stars grow a visible ring around them. Lower it if the target
starts looking soft or plastic.

::: tip When to turn this one off
Structure strength is the setting that can destroy signal, and 0 % switches the
brightness denoiser off altogether. If the target starts looking smeared, waxy, or
like a painting, take it to 0 % before adjusting anything else — the difference is
much easier to judge by switching it off and on than by nudging it.
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

## Streaming Resolution

Two settings decide the size of the image the server sends, and every screen on
the same view receives exactly the same picture:

| Setting | Where | Sets the size for | Choices |
|---|---|---|---|
| **Streaming Resolution** | Settings → Preview | the live view `/` and `/eyepiece` | 1080p, 1440p, 4K, Native |
| **Eyepiece Streaming Resolution** | Settings → Eyepiece | `/eyepiece_quality` | 1440p, 4K, Native |

Both start at **1440p**, and the image is fitted inside the chosen size with its
shape kept — a square sensor at 1440p arrives as 1440×1440. Neither ever goes
above the Processing Resolution, and a camera smaller than the choice is sent at
its own size.

- **Match the screen you look at.** The server shrinks the image with an area
  average, which also removes noise. A larger image left for the browser to shrink
  looks grainier and shimmers through an eyepiece lens.
- **Bigger costs everyone.** Every screen on that view pays for the size in
  bandwidth, and the server in encoding time. A view nobody has open costs nothing.
- **A change applies to the next frame** the server renders — including the frame
  of an exposure that was already running when you changed it.

::: warning After upgrading
Older versions picked the size from each screen. A 4K tablet on the live view now
receives 1440p until you raise **Streaming Resolution**.
:::

## Not applied to planetary targets

Both filters are skipped automatically for **Planetary** stacking. Lucky imaging
exists to recover exactly the fine detail a denoiser removes, so smoothing the
result would undo the whole point of the mode.

## What this does not fix

Coloured dots in the background are hot pixels, not noise — they are in the same
place in every frame, and no amount of smoothing removes a defect that does not
average away. They are removed on the raw sensor data instead; if some survive,
lower the **Hot Pixel Threshold** under [Sensor Corrections](/sensor-corrections#hot-pixels).

Similarly, soft banding across the frame is a readout pattern; it needs
**Row/Column Pattern Removal**, and smoothing it only turns sharp bands into
soft streaks, which is worse.
