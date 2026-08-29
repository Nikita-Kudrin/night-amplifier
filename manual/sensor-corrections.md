# Sensor Corrections

These three settings run on the raw sensor data, before the colour mosaic is
interpolated into RGB. They exist because **more frames will never remove these
defects**: a hot pixel and a readout offset land in exactly the same place in
every sub-exposure, so averaging leaves them precisely where they were.

Find them under **Settings → Sensor**.

## Hot Pixel Rejection

Some sensor pixels read far too bright regardless of what light hits them. On a
2-second, gain-300 frame from an IMX533 there are several thousand of them.
After colour interpolation each one has been smeared into a small coloured
cross, which is why they show up as scattered red and blue dots in the
background rather than as white specks.

Turning this on replaces each one with the average of its same-colour
neighbours. It only touches a pixel that is far brighter than *all* of its
neighbours, so stars — which are several pixels wide — are left alone.

**Detection threshold** sets how far above its brightest neighbour a pixel must
sit. The default of 5σ is measured against the frame's own noise, so it adapts
to your exposure and gain. Lower it if dots survive; raise it if star counts
drop or stars start to look soft.

## Row/Column Pattern Removal

Every sensor row and column reads out with its own small brightness offset. It
does not average down with frame count, and on a drifting mount it smears into
the soft banding you may see across a deep stack. This flattens it by levelling
each row and then each column against the frame's own background.

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
