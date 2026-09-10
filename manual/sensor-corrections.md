# Sensor Corrections

These corrections run on the raw sensor data, before the colour mosaic is
interpolated into RGB. They exist because **more frames will never remove these
defects**: a hot pixel and a readout offset land in exactly the same place in
every sub-exposure, so averaging leaves them precisely where they were.

Find them under **Settings → Sensor**.

Row/Column Pattern Removal is one of the six that
[Focus/Finder mode](/getting-started#focus-finder-mode) holds off while you focus; its
switch greys out while it is on and comes back to your value when you turn it off.
Superpixel Debayer is not one of them. Because pattern removal runs before the stack sees
the frame, that mode is unavailable while you are stacking — see
[It is not available while you are stacking](/getting-started#it-is-not-available-while-you-are-stacking).

## Hot Pixels

Some sensor pixels read far too bright regardless of what light hits them. On a
2-second, gain-300 frame from an IMX533 there are several thousand of them.
After colour interpolation each one has been smeared into a small coloured
cross, which is why they show up as scattered red and blue dots in the
background rather than as white specks.

Night Amplifier always replaces each one with the average of its same-colour
neighbours. It only touches a pixel that is far brighter than *all* of its
neighbours, so stars — which are several pixels wide — are left alone. The
exception is a very sharp star, under about 3 pixels across: its core can land on
a single sensor pixel and be read as a hot one, losing a little of one colour. On
a guide scope imaging at 2 pixels that happened to about 6 % of stars, all of
them faint.

There is no switch to turn this off, in Focus/Finder mode or anywhere else. That
smeared cross is about the size of a real star, and on a short, high-gain guide
exposure there can be several times more of them than stars. Push-To's plate
solver cannot tell them apart and fails on every frame, whatever the field of
view: one such frame read 72 "stars" with the hot pixels left in and 25 with
them removed — and only the cleaned one solved.

**Hot Pixel Threshold** sets how far above its brightest neighbour a pixel must
sit, from 3σ to 12σ. The default of 5σ is measured against every frame's own
noise, so it follows exposure, gain and sky changes from one frame to the next.
Lower it if dots survive; raise it if star counts drop or stars start to look
soft.

## Row/Column Pattern Removal

Every sensor row and column reads out with its own small brightness offset. It
does not average down with frame count, and on a drifting mount it smears into
the soft banding you may see across a deep stack. This levels each row, and then
each column, against its immediate neighbours.

Against its *neighbours*, not against the whole frame — and that distinction is
the whole reason the correction is safe to leave on. A readout offset differs
from one line to the next; a real gradient, or a target spanning hundreds of
lines, changes only gradually. Levelling each line against the frame as a whole
would remove both, and on the reference frame that drained 5 % of the Dumbbell's
brightness along with the banding.

It is skipped automatically for **Planetary** targets: a bright lunar or
planetary disc fills enough of each line to move its measured level, and
flattening that would carve bands across the disc.

## Superpixel Debayer

Normally each pixel's two missing colours are interpolated from its neighbours.
Superpixel mode instead turns each 2x2 group of sensor pixels into one full
colour pixel — no interpolation at all.

- It invents no colour noise, because nothing is interpolated.
- Any hot pixel that survives the filter above stays a single dot instead of
  spreading into a cross.
- It halves both the width and the height.

That last point decides whether it is worth using. A large sensor that already
produces more pixels than your screen loses nothing: an IMX533's 3008x3008
becomes 1504x1504, still more than a 1440x1440 eyepiece display can show. A
smaller sensor does lose real detail — an IMX464's 2712x1538 becomes 1356x769 —
so this is off by default. Try it and judge at the eyepiece.
