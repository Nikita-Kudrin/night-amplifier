# Getting Started

Welcome to Night Amplifier! 

This manual will guide you through setting up and using the software for your EAA sessions.

## Quick Start
1. Connect your astronomy camera.
2. Open Night Amplifier.
3. Select your camera in the UI and click "Connect".
4. Adjust cooling settings if your camera supports it.
5. Click "Start Capture" to begin live stacking.

## Guide camera

You can attach a second camera on a guide scope. Click the small arrow at the right of
**Connect** and choose **As guide camera**.

At most one imaging camera and one guide camera are connected at a time. Connecting
another into a position that is already taken replaces the camera there, unless it is
capturing or warming up — those are refused, so stop them first.

What changes once a guide camera is attached:

- **It runs on its own.** The guide camera starts exposing as soon as it connects and
  keeps going whether or not a capture is running, so you can frame and plate solve
  before pressing Start.
- **It does the plate solving.** With a guide camera attached it is the only camera
  offered to the solver. Set its focal length under **Settings → Equipment** while it is
  the selected camera — a guide scope is usually much shorter than the imaging scope,
  and the solver needs the right field to search.
- **Each camera has its own exposure, gain and cooling.** Click a camera in the list to
  select it; the capture controls then show and edit *that* camera's values, and its
  temperature and dew heater are reported and driven independently of the imaging
  camera's.
- **Start and Stop act on the selected camera.** With the imaging camera selected they
  run the capture session as always. With the guide camera selected they start and stop
  its loop instead — which is how you stop it saving raw frames without disconnecting
  it. The two are independent: stopping the capture leaves the guide camera running, and
  vice versa. Stopping the guide camera hands plate solving back to the imaging one.
- **It has no capture mode.** Nothing it produces is stacked, so Wanderer, Stacking and
  the stacking Type are offered only for the imaging camera; the guide camera always
  shows Live view.
- **A "Guide camera" switch appears** next to the zoom controls over the live view. Turn
  it on to watch the guide camera instead of the imaging one. Push-To arrows are drawn
  over whichever view you are on.
- **Its raw frames have their own switch**, under **Settings → Storage → Save Raw
  Frames**, and go to a folder of their own ending `-guide`. Turning the switch off stops
  the writing on the next frame; so does stopping the camera. A stop ends that folder —
  starting again opens a new one.

The guide camera is only rendered while you are looking at it. With the switch off it
still exposes and still solves, but nothing is processed or encoded for the browser.

## Focus/Finder mode

Focusing and hunting for a target want frame rate, not a clean picture. The
**Focus/Finder mode** switch under the capture panel's Color mode trades one for the
other: it holds off the six stages that cost time on every frame and buy nothing at a
focus mask.

- Background Subtraction
- Shadow Saturation Boost
- Row/Column Pattern Removal
- Colour Mottle
- Background Grain
- Dithering

On an IMX464-sized frame that halves the preview stage — 12.8 ms per frame down to
6.6 ms — before counting the sensor corrections and the denoisers, which run elsewhere.

Hot pixel removal is not on the list and keeps running. Finding a target is when Push-To
plate-solves the most, and a frame full of hot pixels does not solve at all — see
[Hot Pixels](/sensor-corrections#hot-pixels).

Their switches under **Settings** grey out while the mode is on, because the mode
remembers what each one was set to. Turn it off once you are focused and every one goes
back to your value — including the ones you had already turned off yourself.

## It is not available while you are stacking

One of the six — Row/Column Pattern Removal — is not a display setting. It runs on the raw
sensor mosaic, so the frame it cleans up is the frame that goes into the stack. Turning it
off part-way through an integration mixes banding into a master that **nothing can clean
afterwards**: the pattern sits in the same place in every frame, which is the whole reason
the correction exists.

So the switch is disabled while you are stacking, and pressing Start while you are focusing
turns the mode off for you and puts your settings back before the first frame lands. It
stays available in **Live view**, which accumulates nothing — and switching it off is never
blocked, whatever the camera is doing.

Superpixel Debayer is deliberately left alone. It is the *cheap* debayer, so forcing it
either way would work against the frame rate this mode exists to buy — set it to
whatever suits your sensor and it stays there.

## If the camera drops out

USB stalls happen — a knocked cable, a hub that browns out, a driver hiccup. When
the camera stops answering, Night Amplifier notices, disconnects it cleanly, and
tries to bring it back on its own. The imaging and guide cameras recover independently,
so one dropping out never blocks the other's recovery.

What you see in the status bar:

| Message | What it means |
|---|---|
| *"… has stopped responding."* | The camera failed several calls in a row and the session was closed. |
| *"Reconnecting to … — attempt 2 of 5."* | Recovery is under way. The gap between attempts grows: 5 s, then 10, 20, 40, up to a minute. |
| *"… is back. Capture resumed with N frames still stacked."* | The camera came back and your capture picked up where it stopped. |
| *"Could not bring … back after 5 attempts."* | Recovery gave up. Check the cable, then reconnect by hand. |

A resumed capture keeps what it had: the same mode (Live, Wanderer, Stacking or
Planetary), the same exposure and gain, the same raw-frame folder, and — the
part that matters on a long target — **the frames already stacked**. A dropout
90 minutes into a session costs you the dropout, not the 90 minutes. Plate
solving resumes by itself on the next frame if a target was set.

Two switches under **Settings → If the camera drops out**:

- **Reconnect automatically** — on by default. Turn it off if you would rather
  handle dropouts yourself.
- **Resume the capture** — on by default. Turn it off to have the camera
  reconnect but leave the capture stopped.

Recovery deliberately does nothing while the camera is warming up for a
disconnect you asked for, and it stops early if you reconnect by hand first.
