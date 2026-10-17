//! The shared exposure loop against a scripted SDK that records every call it receives.

use std::collections::VecDeque;

use super::*;
use crate::camera::types::{AcquisitionMode, ImageFormat};

/// An SDK that answers polls from a script and logs what the loop asked of it.
struct ScriptedSdk {
    info: CameraInfo,
    polls: VecDeque<Poll>,
    calls: Vec<&'static str>,
    fail_apply: bool,
    slow_apply: Duration,
}

impl ScriptedSdk {
    fn new(polls: impl IntoIterator<Item = Poll>) -> Self {
        Self {
            info: CameraInfo {
                name: "Scripted".to_string(),
                max_width: 8,
                max_height: 4,
                ..Default::default()
            },
            polls: polls.into_iter().collect(),
            calls: Vec::new(),
            fail_apply: false,
            slow_apply: Duration::ZERO,
        }
    }

    fn take_calls(&mut self) -> Vec<&'static str> {
        std::mem::take(&mut self.calls)
    }
}

impl SdkExposure for ScriptedSdk {
    fn info(&self) -> &CameraInfo {
        &self.info
    }

    fn apply(&mut self, _config: &CaptureConfig) -> CameraResult<()> {
        self.calls.push("apply");
        std::thread::sleep(self.slow_apply);
        if self.fail_apply {
            return Err(CameraError::ExposureFailed("refused".to_string()));
        }
        Ok(())
    }

    fn start(&mut self, acquisition: Acquisition) -> CameraResult<()> {
        self.calls.push(match acquisition {
            Acquisition::Stream => "start_stream",
            Acquisition::Single => "start_single",
        });
        Ok(())
    }

    fn end_stream(&mut self) {
        self.calls.push("end_stream");
    }

    fn abort(&mut self, acquisition: Acquisition) {
        self.calls.push(match acquisition {
            Acquisition::Stream => "abort_stream",
            Acquisition::Single => "abort_single",
        });
    }

    fn on_stall(&mut self, _progress: &Progress) {
        self.calls.push("on_stall");
    }

    fn frame_len(&mut self, _config: &CaptureConfig) -> CameraResult<usize> {
        Ok(8 * 4)
    }

    fn poll(&mut self, _progress: &Progress, _buffer: &mut [u8]) -> Poll {
        self.calls.push("poll");
        self.polls.pop_front().unwrap_or(Poll::Pending)
    }

    fn finish_single(&mut self) -> CameraResult<()> {
        self.calls.push("finish_single");
        Ok(())
    }
}

fn ready() -> Poll {
    Poll::Ready { width: 8, height: 4 }
}

fn video(exposure_us: u64) -> CaptureConfig {
    CaptureConfig {
        exposure_us,
        acquisition: AcquisitionMode::Video,
        format: ImageFormat::Raw8,
        ..Default::default()
    }
}

fn snap(exposure_us: u64) -> CaptureConfig {
    CaptureConfig {
        exposure_us,
        acquisition: AcquisitionMode::Snap,
        format: ImageFormat::Raw8,
        ..Default::default()
    }
}

#[test]
fn a_stream_is_configured_and_started_once_then_only_read() {
    let mut sdk = ScriptedSdk::new([ready(), ready()]);
    let mut exposure = ExposureLoop::new();

    let frame = exposure.capture(&mut sdk, &video(1_000)).unwrap();
    assert_eq!((frame.width, frame.height), (8, 4));
    assert_eq!(sdk.take_calls(), ["apply", "start_stream", "poll"]);

    exposure.capture(&mut sdk, &video(1_000)).unwrap();
    assert_eq!(sdk.take_calls(), ["poll"], "same config, stream already running");
}

/// A self-describing frame keeps the size and depth it arrived with, whatever the
/// config asked for — INDI's driver, not the config, decides what a FITS BLOB holds.
#[test]
fn a_delivered_frame_is_returned_as_it_came() {
    let delivered = RawFrame {
        data: vec![1, 0, 2, 0].into(),
        width: 2,
        height: 1,
        format: ImageFormat::Raw16,
    };
    let mut sdk = ScriptedSdk::new([Poll::Pending, Poll::Delivered(delivered)]);

    let frame = ExposureLoop::new().capture(&mut sdk, &snap(1_000)).unwrap();
    assert_eq!((frame.width, frame.height, frame.format), (2, 1, ImageFormat::Raw16));
    assert_eq!(&frame.data[..], &[1, 0, 2, 0]);
}

#[test]
fn a_changed_config_stops_the_stream_before_it_is_applied() {
    let mut sdk = ScriptedSdk::new([ready(), ready()]);
    let mut exposure = ExposureLoop::new();
    exposure.capture(&mut sdk, &video(1_000)).unwrap();
    sdk.take_calls();

    exposure.capture(&mut sdk, &video(2_000)).unwrap();
    assert_eq!(sdk.take_calls(), ["end_stream", "apply", "start_stream", "poll"]);
}

/// A config the camera refused is not remembered as applied, so the next capture tries
/// it again instead of exposing with whatever the camera kept.
#[test]
fn a_refused_config_is_applied_again_next_time() {
    let mut sdk = ScriptedSdk::new([ready()]);
    sdk.fail_apply = true;
    let mut exposure = ExposureLoop::new();

    assert!(exposure.capture(&mut sdk, &snap(1_000)).is_err());
    sdk.fail_apply = false;
    exposure.capture(&mut sdk, &snap(1_000)).unwrap();
    assert_eq!(sdk.take_calls(), ["apply", "apply", "start_single", "poll", "finish_single"]);
}

#[test]
fn invalidating_pushes_the_same_config_again() {
    let mut sdk = ScriptedSdk::new([ready(), ready()]);
    let mut exposure = ExposureLoop::new();
    exposure.capture(&mut sdk, &snap(1_000)).unwrap();
    exposure.invalidate();
    sdk.take_calls();

    exposure.capture(&mut sdk, &snap(1_000)).unwrap();
    assert_eq!(sdk.take_calls(), ["apply", "start_single", "poll", "finish_single"]);
}

/// Single exposures and a free-running stream do not mix: switching ends the stream, and
/// switching back starts it again.
#[test]
fn switching_to_single_exposures_ends_the_stream() {
    let mut sdk = ScriptedSdk::new([ready(), ready(), ready()]);
    let mut exposure = ExposureLoop::new();
    let mut stream = video(1_000);
    stream.acquisition = AcquisitionMode::Auto;
    let mut single = stream.clone();
    single.acquisition = AcquisitionMode::Snap;

    exposure.capture(&mut sdk, &stream).unwrap();
    exposure.capture(&mut sdk, &single).unwrap();
    exposure.capture(&mut sdk, &stream).unwrap();

    assert_eq!(
        sdk.take_calls(),
        [
            "apply", "start_stream", "poll",
            "end_stream", "apply", "start_single", "poll", "finish_single",
            "apply", "start_stream", "poll",
        ]
    );
}

#[test]
fn polling_carries_on_until_the_frame_is_ready() {
    let mut sdk = ScriptedSdk::new([Poll::Pending, Poll::Pending, ready()]);
    let mut exposure = ExposureLoop::new();

    exposure.capture(&mut sdk, &snap(1_000)).unwrap();
    assert_eq!(sdk.take_calls().iter().filter(|c| **c == "poll").count(), 3);
}

/// A cancel arriving while the frame is awaited aborts that exposure, and a stream it
/// aborted is started again by the next capture.
#[test]
fn a_cancel_aborts_the_exposure_in_flight() {
    for (config, abort) in [(video(1_000), "abort_stream"), (snap(1_000), "abort_single")] {
        let mut sdk = ScriptedSdk::new([]);
        let mut exposure = ExposureLoop::new();
        let token = exposure.cancel_token();
        let cancel = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            token.store(true, Ordering::SeqCst);
        });

        let outcome = exposure.capture(&mut sdk, &config);
        cancel.join().unwrap();

        assert!(matches!(outcome, Err(CameraError::Cancelled)));
        assert_eq!(sdk.take_calls().last(), Some(&abort));
    }

    let mut sdk = ScriptedSdk::new([ready()]);
    let mut exposure = ExposureLoop::new();
    exposure.cancel();
    assert!(exposure.capture(&mut sdk, &video(1_000)).is_ok(), "a capture starts uncancelled");
}

/// The stall hook runs before the abort, while the SDK still has its counters.
#[test]
fn a_stalled_exposure_times_out_and_is_aborted() {
    let mut sdk = ScriptedSdk::new([]);
    let mut exposure = ExposureLoop::new();
    let config = video(1_000);
    let budget = config.stall_budget(8 * 4);

    let started = Instant::now();
    let outcome = exposure.capture(&mut sdk, &config);

    assert!(matches!(outcome, Err(CameraError::ExposureTimeout(b)) if b == budget));
    assert!(started.elapsed() >= budget);
    let calls = sdk.take_calls();
    assert_eq!(&calls[calls.len() - 2..], ["on_stall", "abort_stream"]);

    sdk.polls.push_back(ready());
    exposure.capture(&mut sdk, &config).unwrap();
    assert_eq!(sdk.take_calls(), ["start_stream", "poll"], "the stream starts again");
}

/// The SDK knows whether its failure took the stream down; a stream it kept is read on,
/// a stream it lost is started again.
/// The budget runs from entering `capture`, config reapply included — the instant the
/// watchdog starts its clock. Timed from after the reapply, a slow reapply pushed the
/// stall past the watchdog, which then abandoned the handle instead of one retry.
#[test]
fn a_slow_reapply_spends_the_frames_own_budget() {
    let mut sdk = ScriptedSdk::new([]);
    sdk.slow_apply = Duration::from_millis(800);
    let config = video(1_000);
    let budget = config.stall_budget(8 * 4);

    let entered = Instant::now();
    let outcome = ExposureLoop::new().capture(&mut sdk, &config);

    assert!(matches!(outcome, Err(CameraError::ExposureTimeout(_))));
    assert!(
        entered.elapsed() < budget + Duration::from_millis(400),
        "timed out {:?} after entering, past the {budget:?} budget",
        entered.elapsed()
    );
}

#[test]
fn a_failed_poll_ends_the_stream_only_when_the_sdk_says_so() {
    let next_capture = |stream_ended: bool| {
        let failed = Poll::Failed {
            error: CameraError::Disconnected,
            stream_ended,
        };
        let mut sdk = ScriptedSdk::new([failed, ready()]);
        let mut exposure = ExposureLoop::new();
        let outcome = exposure.capture(&mut sdk, &video(1_000));
        assert!(matches!(outcome, Err(CameraError::Disconnected)));
        sdk.take_calls();

        exposure.capture(&mut sdk, &video(1_000)).unwrap();
        sdk.take_calls()
    };

    assert_eq!(next_capture(true), ["start_stream", "poll"]);
    assert_eq!(next_capture(false), ["poll"]);
}

#[test]
fn an_invalid_config_never_reaches_the_sdk() {
    let mut sdk = ScriptedSdk::new([ready()]);
    sdk.info.max_exposure_us = 1_000;
    let mut exposure = ExposureLoop::new();

    assert!(exposure.capture(&mut sdk, &snap(5_000)).is_err());
    assert!(sdk.take_calls().is_empty());
}
