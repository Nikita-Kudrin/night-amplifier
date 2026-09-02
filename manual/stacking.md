# Live Stacking

Night Amplifier supports different stacking methodologies based on your celestial target.

## Deep Sky Stacking
Traditional star-based alignment. The software detects stars, matches triangles across frames using RANSAC, and calculates an affine transformation to align the frames.

## Planetary Stacking
(Also known as Lucky Imaging). This mode uses correlation-based alignment for high-framerate planetary or lunar targets where stars are absent. It uses percentile stacking (e.g., top 10% of frames) based on sharpness metrics.

## Comet Stacking (Pro)
Uses an ROI (Region of Interest) around the comet's nucleus to align frames on the moving comet, while aggressive rejection algorithms drop trailing stars.

## Frame Rejection

Not every captured frame is worth adding to the stack. A frame that arrives during a gust, a
passing cloud, or a bump of the mount will either fail to align at all or align badly, and
averaging a badly aligned frame in smears every star in the result.

Night Amplifier decides per frame, and reports the outcome in the status bar as
`Rejected N | Total M`. Hovering the info icon next to the counter names the most recent
reason. A frame is dropped when:

- too few stars are detected to attempt an alignment;
- no alignment can be found against the reference;
- the alignment rests on too small a fraction of the detected stars to be trusted;
- its alignment error is far worse than the rest of the session's;
- its stars are far larger than the rest of the session's — defocus, cloud, or shake.

The last two thresholds follow your session rather than a fixed standard: they are derived from
the frames measured so far, so a night of soft seeing is not rejected wholesale, while a single
bad frame on a night of good seeing still is. The first few frames of a session are always
accepted, since there is nothing yet to compare them against.

Alignment error is also judged against the size of your stars, not only against the rest of the
session. A misalignment matters in proportion to what it is smearing: a pixel of error is nothing
on a soft, wide star and obvious on a tight one. Without that, a well-tracking mount ended up with
the strictest gate of all, throwing away frames that were aligned to well inside a single star.

During those first frames the software also watches for a sharper frame than the one it started
on. The reference frame sets the sharpness ceiling for everything stacked onto it, and the first
frame of a session is chosen before anything is known about the night. If a frame arrives early
that is clearly sharper — sharper than the reference *and* than the session so far, so that it is
a real change in conditions rather than the ordinary frame-to-frame wobble in the measurement —
the stack restarts from it. The frame counter resets, and the few seconds of integration given up
buy a sharper result for the rest of the session.

The live view always shows the accumulated stack. A rejected frame leaves the stack as it was, so
the preview holds steady instead of dropping back to a single noisy sub-exposure.

### Wanderer mode

Wanderer mode restarts the stack when you swing the telescope to a new object, which it detects by
the incoming field no longer matching the reference. Frames that the checks above drop for
*quality* — soft stars, a loose fit — do not count: the field is still the same field, so the stack
keeps building and rides out the cloud rather than starting over.

## Saving raw frames

Raw sub-exposures are saved under **Settings → Storage → Save Raw Frames**, which lists the three
capture modes separately. Live view, Wanderer and Stacking are chosen independently, so you can
keep the subs from a Wanderer sweep without also filling the card during every focusing run. All
three start off.

Deep Sky and Comet write one FITS file per exposure. Planetary writes a single SER container per
session instead, in every mode — a planetary Live view run records the same way a planetary
stacking run does.

Each session writes to its own folder under `captures/raw/`, named for the time it started and the
mode that filled it — `21-14-08-live`, `21-31-52-wanderer`, `22-03-17-stacking`. Switching mode
without stopping the capture opens a new folder, so a folder only ever holds frames captured in the
mode it names. Two sessions starting inside the same second get a counter before the suffix
(`21-14-08_2-stacking`) rather than sharing a folder.

Frame numbers run for the whole capture, not per folder, so a folder opened by a mode switch starts
partway through the sequence — `frame_000517.fits` rather than `frame_000001.fits`.

Saving in Live view is worth knowing about before you turn it on: it writes one file per exposure,
and at the short exposures used for focusing that is a great many files very quickly. If the disk
cannot keep up, frames are dropped rather than made to wait — the capture never stalls, but the set
on disk will have gaps.

**Save Stacked Image** stays Stacking-only. Live view builds no stack, and Wanderer discards its
stack every time the telescope moves, so there is no single result to write.
