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

Both downloads come from the server at the frame's own resolution, not at the
[streaming resolution](/noise-reduction#streaming-resolution), so they are worth keeping. That render is
big enough that only one runs at a time: if somebody on another device is already
saving one, the button keeps spinning and retries for up to fifteen seconds before
telling you the server is busy.

## Running it as a dedicated display

A tablet or a small panel left on `/eyepiece_quality` all night needs nothing from
you once it is up:

- The view **reconnects on its own, indefinitely**, backing off from one second to
  thirty. Restarting Night Amplifier mid-session no longer leaves a black screen
  that only a page reload can clear — which matters when the display has no keyboard.
- It shows the **last rendered frame the moment it connects**, rather than waiting
  for the next exposure. At 60-second subs that used to be a minute of black, and a
  display opened after capture had stopped stayed black indefinitely.
- Its size is **Settings → Eyepiece → Eyepiece Streaming Resolution**, not the
  display's own: set it to match the panel — 1440p for a 1440×1440 screen (see
  [Streaming Resolution](/noise-reduction#streaming-resolution)).

Launch it in kiosk mode from your startup script, waiting for the server first —
Chromium will not retry a page that failed to load:

```bash
until curl -sf -o /dev/null http://localhost:8080/; do sleep 0.5; done
chromium --kiosk 'http://localhost:8080/eyepiece_quality' \
    --noerrdialogs --disable-infobars --no-first-run \
    --ozone-platform-hint=auto --password-store=basic
```

Upgrading the binary does not need the browser cache cleared. The interface is
served with validators, so Chromium re-checks `index.html` on every load and reuses
the fingerprinted bundle only while it is still the current one.

## Black level

How dark the background sky is pushed. Raising it darkens the sky and lifts the
contrast of the target — and it also pushes more of the sky's noise below black,
so the background looks smoother as well as darker.

This slider is not the only thing that calms the sky any more. A deeper stack
does it too: the auto-stretch now spends part of what stacking buys on a quieter
background and the rest on the target, instead of all of it on the target. Sixteen
frames render about half the grain of one, sixty-four about a third, and the target
keeps growing the whole time. It stops there: past sixty-four frames a real
session's noise no longer falls fast enough to pay for both, so everything beyond
that goes to the target.

::: warning Changed behaviour
Two things moved here. The slider used to push the black point the *other* way,
so raising it made the sky grainier and clipped more of it to pure black — the
opposite of what it described. And deep stacks used to look exactly as grainy as
single frames. If you kept this slider high to get a smooth sky out of an earlier
version, expect a darker, smoother one at the same setting now, and turn it down
if the faint outskirts of your target have gone.
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
| 0 % | 11 and 17 levels | — |
| −5 % | 4 and 6 levels (−64 to −65 %) | +32 % and 0 % |
| −6 % (end stop) | 3 levels (−73 to −82 %) | +31 % and +14 % |
| Black level at full | 7 and 8 levels (−50 %) | −62 % |

Those are code values. What reaches your eye falls further, because the panel
applies its own gamma on top.

It works by dimming the sky, not by cutting it off: each pixel is darkened
according to its small neighbourhood, so flat sky darkens evenly while stars and
the faint glow of a nebula or globular cluster keep their level. The grain in the
sky shrinks with it, instead of turning into dark clumps and bright specks.

The setting follows the sky it measures, not full scale, so one position behaves
the same on a bright target and a faint one. A large dark area in the frame — a roof,
a tree, a dewed-over corner — is not mistaken for the sky. It does nothing with **Auto stretch**
off, and nothing in **Planetary** mode — there the middle of the frame is the Moon
or the planet rather than sky, so it would darken the subject instead.

### Darker sky

The negative half never switches a pixel fully off. **Darker sky** replaces the
dimming with a hard cut at the chosen level.

It buys the deepest possible background, and it costs the black speckle the
positive half of this slider exists to remove — a third to a half of the sky ends
up fully off. Worth trying on an LCD, or on a target bright enough that you do not
care what happens to the background. It does nothing while Black floor is positive.

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
