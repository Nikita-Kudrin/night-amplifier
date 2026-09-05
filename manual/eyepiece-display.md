# Eyepiece Display

Three settings decide what the darkest part of the image does when it reaches
your screen. They matter far more at the eyepiece than on a desk monitor,
because an OLED panel a few centimetres from your eye shows you every individual
pixel.

Find them under **Settings → Processing**.

## The eyepiece view

Open `/eyepiece` for the view itself. It is always a single image, whatever
**Binoview** is set to — the split-screen layout lives on `/eyepiece_quality`,
which still follows the setting.

Its controls sit in the bottom-right corner and **fade out after ten seconds**,
so nothing stands between you and the sky. Tap or click the image to bring them
back, and tap again to dismiss them early. Push-To chevrons are exempt: they stay
on screen while you are navigating to a target.

| Control | What it does |
|---|---|
| Fullscreen | Fills the screen, and fits the image to it — leaving fullscreen fits it again to the smaller viewport. Rotating the device or resizing the window re-fits while fullscreen; windowed, your own zoom is left alone. Hidden on iPhone, which has no fullscreen for web pages. |
| Fit all | Appears once you have pinched or scrolled in. Returns to the whole frame. |
| Download | Saves the round eyepiece image: a square PNG, black outside the field stop — the view as you were looking at it. |
| Download original | On the button's dropdown. The same picture uncropped — the full rectangular frame. |

Both downloads come from the server at the frame's own resolution, not at
whatever size your screen is streaming, so they are worth keeping. That render is
big enough that only one runs at a time: if somebody on another device is already
saving one, the button keeps spinning and retries for up to fifteen seconds before
telling you the server is busy.

## Black level

How dark the background sky is pushed. Raising it darkens the sky and lifts the
contrast of the target — and it also pushes more of the sky's noise below black,
so the background looks smoother as well as darker.

That last part is worth knowing about, because it is the *only* thing in the
stretch that changes how grainy the sky looks. More frames in the stack will not
do it: the auto-stretch solves for a fixed background level, so as stacking
lowers the noise the stretch amplifies harder by exactly the compensating amount.
Thirty-five frames look as grainy as one. Depth in the target is what stacking
buys you; smoothness comes from this slider and from
[Noise Reduction](/noise-reduction).

::: warning Changed direction
This slider used to move the black point the *other* way, so pushing it up made
the sky grainier and clipped more of it to pure black — the opposite of what it
described. If you had it set high from an earlier version, expect a darker,
smoother sky at the same setting now, and turn it down if the faint outskirts of
your target have gone.
:::

## Black floor

Sets where black sits. The slider runs both ways from zero, and the two
directions solve opposite problems.

### Positive: keeping the darkest pixels lit

The black point sits below the sky level by design, so a few per cent of sky
pixels land at exactly zero — 0.8 % on the reference frame. On an LCD they are
just very dark. On an OLED they are switched fully off, and at eyepiece
magnification each one is large enough for your eye to resolve on its own, so
they read as hard black speckle scattered through a grey sky rather than as sky.

The floor lifts everything just clear of that. The default is 4 % of full scale,
which is around output level 10 — dark enough to still read as black, bright
enough that the panel keeps the pixel lit.

Raise it if the background shows hard black dots. Lower it for maximum contrast
on a screen that does not switch pixels off.

### Negative: a darker sky

With the floor at zero the sky still sits at 14 to 17 output levels, which
through an eyepiece lens is a clearly visible grey rather than a night sky.
Turning **Black level** down darkens it, but that works by weakening the stretch,
so the target dims with the background: at full travel it takes the sky down by
half and the target's contrast down by nearly two thirds.

The negative half of Black floor lowers the background *without* touching the
stretch, so the target keeps its brightness. Measured on the reference frames:

| Setting | Sky | Target contrast |
|---|---|---|
| 0 % | 14 and 17 levels | — |
| −5 % | 4 and 6 levels (−65 to −71 %) | +9 % and +25 % |
| −9 % | 2 levels (−86 to −88 %) | −38 % and −1 % |
| Black level at full | 7 and 8 levels (−50 %) | −62 % |

Those are code values. What reaches your eye falls further, because the panel
applies its own gamma on top: 14 levels down to 4 is a 72 to 94 % drop in
emitted light depending on the screen.

The setting is anchored to the sky it measures, not to full scale, so one
position behaves the same on a bright target and a faint one. Around −5 % puts
the floor level with the sky itself. Past that you are cutting into the sky's own
noise, which is what makes the last of the travel cost some faint detail.

Because it is anchored to a measured sky level, the negative half needs something
to measure. It does nothing with **Auto stretch** off, and nothing in **Planetary**
mode — there the middle of the frame is the Moon or the planet rather than sky, so
a floor set from it would darken the subject instead of the background.

### Darker sky

The negative half rolls off into black rather than clipping, so no pixel is ever
switched fully off. **Darker sky** removes that roll-off and lets the sky clip.

It buys the deepest possible background and a little more separation between
target and sky, and it costs the black speckle the positive half of this slider
exists to remove — around a third of the sky ends up fully off. Worth trying on
an LCD, or on a target bright enough that you do not care what happens to the
background. It does nothing while Black floor is positive.

## Dithering

Breaks up the steps between brightness levels before the image is reduced to
8 bits.

**You will probably see no difference, and that is the expected result.** A sky
with visible grain already dithers itself — the noise is doing the job. This
matters once the background is genuinely smooth, which is what
[Noise Reduction](/noise-reduction) is for: a smoothed low-slope gradient
quantised to 256 levels is exactly what shows banding, and this is what prevents
it. It costs nothing to leave on, so leave it on and judge it after the
denoisers, not before.

## What these do not fix

Black speckle is the black point clipping; hard *coloured* dots are hot pixels,
which are in the same place in every frame and want
[Sensor Corrections](/sensor-corrections) instead. Neither the floor nor the
dither removes grain — that is
[Noise Reduction](/noise-reduction), and above all its **Star protection**
control.
