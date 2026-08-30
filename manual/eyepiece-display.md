# Eyepiece Display

Three settings decide what the darkest part of the image does when it reaches
your screen. They matter far more at the eyepiece than on a desk monitor,
because an OLED panel a few centimetres from your eye shows you every individual
pixel.

Find them under **Settings → Processing**.

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

Keeps the darkest pixels just above pure black.

The black point sits below the sky level by design, so a few per cent of sky
pixels land at exactly zero — 0.8 % on the reference frame. On an LCD they are
just very dark. On an OLED they are switched fully off, and at eyepiece
magnification each one is large enough for your eye to resolve on its own, so
they read as hard black speckle scattered through a grey sky rather than as sky.

The floor lifts everything just clear of that. The default is a little under 2 %
of full scale, which is around output level 10 — dark enough to still read as
black, bright enough that the panel keeps the pixel lit.

Raise it if the background shows hard black dots. Lower it for maximum contrast
on a screen that does not switch pixels off.

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
