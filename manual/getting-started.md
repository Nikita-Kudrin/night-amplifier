# Getting Started

Welcome to Night Amplifier! 

This manual will guide you through setting up and using the software for your EAA sessions.

## Quick Start
1. Connect your astronomy camera.
2. Open Night Amplifier.
3. Select your camera in the UI and click "Connect".
4. Adjust cooling settings if your camera supports it.
5. Click "Start Capture" to begin live stacking.

## If the camera drops out

USB stalls happen — a knocked cable, a hub that browns out, a driver hiccup. When
the camera stops answering, Night Amplifier notices, disconnects it cleanly, and
tries to bring it back on its own.

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
