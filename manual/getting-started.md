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
  camera's. Starting a capture always uses the imaging camera, whichever one is selected.
- **A "Guide camera" switch appears** next to the zoom controls over the live view. Turn
  it on to watch the guide camera instead of the imaging one. Push-To arrows are drawn
  over whichever view you are on.
- **Its raw frames have their own switch**, under **Settings → Storage → Save Raw
  Frames**, and go to a folder of their own ending `-guide`.

The guide camera is only rendered while you are looking at it. With the switch off it
still exposes and still solves, but nothing is processed or encoded for the browser.

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
