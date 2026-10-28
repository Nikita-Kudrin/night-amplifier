# Live Stacking

Night Amplifier supports different stacking methodologies based on your celestial target.

## Deep Sky Stacking
Traditional star-based alignment. The software detects stars, matches triangles across frames using RANSAC, and calculates an affine transformation to align the frames. That first fit uses only the brightest stars, so it is then refined against every star detected in both frames, which keeps the corners of a wide field as sharp as its centre.

Short sub-exposures of a bright object work too. Noise on a bright nebula breaks it into dozens of false "stars", brighter than the real ones, so the matcher keeps only the brightest detection in each patch of sky. If that still finds no alignment, the software looks for the one shift that most star pairs agree on, trying a few small turns as well, since the field of an alt-az mount slowly rotates. That is enough for a handful of real stars among hundreds of detections: 0.7 s subs of the Cat's Eye Nebula hold about 15. A shift found this way has to stand out from all the shifts it was picked from, so a different patch of sky is not mistaken for this one. Longer exposures or more gain still give more stars to align on.

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
- every star in it is doubled or smeared into a line — the mount moved during the exposure.
  This is only judged on frames that would not align anyway, so it changes the reason
  reported, not how many frames are kept;
- the alignment is no better than chance: too few stars line up for where they are (a dense
  patch lines some up by luck), or it would rescale the field, which a sub of the same session
  never does;
- its alignment error is far worse than the rest of the session's;
- its stars are far larger than the rest of the session's — defocus, cloud, or shake.

The last two thresholds follow your session rather than a fixed standard: they are derived from
the frames measured so far, so a night of soft seeing is not rejected wholesale, while a single
bad frame on a night of good seeing still is. The first frames are judged once there are enough
of them to judge by — see below.

Alignment error is also judged against the size of your stars, not only against the rest of the
session. A misalignment matters in proportion to what it is smearing: a pixel of error is nothing
on a soft, wide star and obvious on a tight one. Without that, a well-tracking mount ended up with
the strictest gate of all, throwing away frames that were aligned to well inside a single star.

### The first six frames

Every check above compares a frame with the session so far, so the first frames of a stack have
nothing to be judged against, and the very first one becomes the reference everything else is
aligned to. A bumped, trailed, smeared or clouded first frame used to stay in charge all night:
a bumped one left every star of the stack about 40 % larger.

So the first six frames stack as usual, and the live view shows them, but the stack also keeps a
copy of each. Once the sixth arrives they are judged together. The reference becomes the frame the
others line up best on, and stays the first one unless another is clearly better. Frames whose
stars line up far worse than the rest — cloud, a bump, a trail — are taken out, the remaining
frames are judged by the checks above against each other, and the stack is rebuilt from what
remains, each frame weighted against the whole set. The counters move the frames taken out from
*stacked* to *rejected*, and a frame first refused against a bad reference can come back in.

If no frame lines up with most of the others, nothing is rebuilt and the stack carries on as it
streamed; a clearly sharper frame among the first ten can then still restart it, as it could before.

The rebuild takes a moment once (about a third of a second on a desktop PC, longer on a Raspberry
Pi), and the six copies hold about 330 MB at 3008×3008 in colour until it is done; on a computer
short of free memory fewer frames are held, or none. A stack restarted by Wanderer mode starts its
own six. Smeared stars are not caught: a frame drawn out by wind looks
much like the ordinary sub-to-sub wobble of an alt-az field, and stays in the stack as it would
mid-session.

The live view always shows the accumulated stack. A rejected frame leaves the stack as it was, so
the preview holds steady instead of dropping back to a single noisy sub-exposure.

### Wanderer mode

Wanderer mode restarts the stack when you swing the telescope to a new object, which it detects by
the incoming field no longer matching the reference. Frames that the checks above drop for
*quality* — soft stars, doubled stars, a loose fit — do not count: the field is still the same
field, so the stack keeps building and rides out the cloud, or the bump, rather than starting over.
Five frames in a row whose stars are swollen *and* will not align are the exception: that is no
passing gust but a new field (or a new eyepiece train), and the stack restarts.

One frame that will not align is not enough to restart: it takes two in a row. A faint field's
frames align on only a few stars, and a single weak one should not throw away the stack. A real
slew costs one extra frame before the new stack begins.

### Auto crop

The stack keeps the first frame's framing, so drift, field rotation on an alt-az mount or a bumped
tripod leave a border that fewer frames reached: noisier than the rest, with seams where each
frame's edge falls. **Settings → Stacking → Auto Crop** (Deep Sky, on by default) cuts the stack
to the largest rectangle that nearly every frame covers. It starts at 20 frames, the first depth
at which one bumped frame is outvoted by the rest; until then the stack is shown whole. The cut
applies to the live view, both eyepiece views and the saved FITS and PNG. The FITS records where
its corner sat in the full frame as `XORGSUBF`/`YORGSUBF`. Plate solving still sees the whole frame.

The crop is applied before the image is stretched. Left in, the thin border made the whole sky
measure noisier than it is, and the stretch came out flatter. As the field keeps drifting the crop
shrinks, in occasional steps rather than on every frame. If you have zoomed in, the live view keeps
the same patch of sky in view across a step. A step leaves the picture's brightness alone: cutting
sky away used to make a bright nebula read as noisier and dim it a little at each step, through to
the saved PNG. With the setting off nothing is ever cut.

## Background removal

**Settings → Processing → Background Subtraction** (on by default) removes the glow light pollution, the
moon, the horizon and uncorrected vignetting lay over the sky, including a glow creeping in from
one edge or a corner. The galaxy, nebula or cluster you are looking at is kept out of the
background model: anything that brightens *inside* the frame is treated as a target, anything
that brightens *towards an edge* as glow. The whole gradient is removed in every stretch mode.

Narrow bands along an edge — stray light inside the tube, the edge of an uncorrected vignette —
are removed from the stacked picture too: they run the whole length of the edge, which a target
touching that edge does not. Live view, showing single frames, leaves them in. Two things along
an edge are left alone: an object the plate solve found there (Pro), even when it runs the edge's
whole length, and the thin border a young stack has where the frames have drifted, until
**Auto crop** cuts it at 20 subs.

**Algorithm** picks how the sky samples are joined: Grid, or RBF (Pro), a smooth spline with no
seams between samples. Pro starts on RBF; if the licence lapses, the picture falls back to Grid
until it is renewed, and your choice is kept. Pro also starts with **Shadow Saturation Boost**
on, at 10 %.

With Pro and plate solving installed, the stack is also plate solved once, in the background,
after its first few frames: every catalogued object in the field is then protected by its
catalogued size, even a faint nebula that a glow from the same edge would otherwise hide. The
picture never waits for it, and Push-To always comes first: a Push-To solve pauses it until a
later frame. With a guide camera, the stack is still solved through the imaging camera's own
optics. A field the solver cannot recognise within 90 seconds (very zoomed in, few stars)
simply carries on without it; a new stack, after a slew or a Wanderer restart, is solved again.

Limits: a glow that peaks in the middle of the frame (a passing cloud) is kept as if it were a
target, and a nebula touching the frame's edge without a catalogue entry can be partly removed
with the glow beside it. Noise Reduction does not remove gradients, but by smoothing the grain it
makes any that remain easier to see.

## Outlier Rejection (Pro)

Satellite trails, aircraft and cosmic-ray hits are removed pixel by pixel: each new sample is
compared with the spread that pixel has shown so far and clipped when it lies too far out. Pro
starts on **Sigma Clipping**, with **Winsorized** as the gentler alternative; Community averages
every sample. Deep-sky and planetary stacking both run the method you pick.

A pixel's first few samples have no spread to be compared with, so a trail in the first frames of
a deep-sky stack used to stay in it. When the first six frames are rebuilt (see *The first six
frames*), each pixel's six samples are compared with each other instead, and a trail crossing one
of them is removed before any of it is stacked.

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
(`21-14-08_2-stacking`) rather than sharing a folder. A folder only appears once its first frame is
saved, so a run that saves nothing — a mode whose switch is off, or one stopped before its first
sub — leaves no empty folder behind. A disk that will not take writes (read-only or full) refuses
the capture's start; one that stops taking them mid-session raises a single notice rather than
losing the rest of the night quietly.

Frame numbers run for the whole capture, not per folder, so a folder opened by a mode switch starts
partway through the sequence — `frame_000517.fits` rather than `frame_000001.fits`.

Saving in Live view is worth knowing about before you turn it on: it writes one file per exposure,
and at the short exposures used for focusing that is a great many files very quickly. If the disk
cannot keep up, frames are dropped rather than made to wait — the capture never stalls, but the set
on disk will have gaps.

**Save Stacked Image** stays Stacking-only. Live view builds no stack, and Wanderer discards its
stack every time the telescope moves, so there is no single result to write.

## Before you power off

A file the computer has "saved" can still be waiting in memory for up to half a minute before it
reaches the card or drive, and cutting the power then leaves it empty. So when you press **Stop**,
the status bar first shows **Saving to disk — keep the power on**, while the stack, its PNG, the
raw frames or the SER video are pushed onto the device. Then it shows **Saved N files to disk**,
and it is safe to power off. If something did not make it, it says how many files and why instead.
The guide camera's saved frames get their own notice when the guide camera stops — **Saved N
guide frames to disk** — so wait for both if both were saving. Settings are written the same way
each time you change them.

On a slow card **Stop** can take a while: the camera is free again at once (a disconnect starts
warming it up), but the capture reads *Stopping* until the files are on the card. If the drive stops
responding altogether — unplugged, or a card that hangs — the app gives up after a minute without
progress and says **Captures not confirmed on disk**, rather than staying in *Stopping* all night;
if the stack itself could not be handed over, it says so too.

Shutting the computer down, or stopping the service (`systemctl stop`), during a capture is safe:
the app stops both cameras, saves the stack and syncs everything first, for up to 75 seconds.
Pressing Ctrl-C a second time exits at once.
