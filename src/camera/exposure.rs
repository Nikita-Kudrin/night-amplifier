//! The exposure loop every vendor SDK shares.
//!
//! Apply a config only when it changed, keep a free-running stream or trigger one exposure
//! at a time, and give up on cancel or a stall. A vendor implements [`SdkExposure`] — the
//! calls its SDK makes, quirks included — and [`ExposureLoop`] owns the state machine, so
//! it is tested once, without hardware, for all of them. The stall clock starts on entering
//! `capture`, config reapply included — the instant the watchdog starts its own, which
//! [`CaptureConfig::stall_budget`] has to stay below.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing::info_span;

use super::error::{CameraError, CameraResult};
use super::types::{BufferPool, CameraInfo, CaptureConfig, RawFrame};

/// How a frame is acquired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Acquisition {
    /// A free-running video stream, left running between frames.
    Stream,
    /// One triggered exposure per frame.
    Single,
}

/// Where the exposure in flight stands, for an SDK whose poll depends on it.
pub(crate) struct Progress<'a> {
    pub acquisition: Acquisition,
    pub config: &'a CaptureConfig,
    /// Since the exposure started.
    pub waited: Duration,
    /// How long the frame may take before it counts as stalled.
    pub budget: Duration,
}

impl Progress<'_> {
    pub fn left(&self) -> Duration {
        self.budget.saturating_sub(self.waited)
    }
}

/// One look for the frame.
pub(crate) enum Poll {
    /// The frame is in the buffer, at this size.
    Ready { width: u32, height: u32 },
    /// Not yet. The SDK call waited, or the backend slept, before saying so.
    Pending,
    /// The exposure failed. Any cleanup the SDK needs is already done; `stream_ended`
    /// says whether a running stream went with it.
    Failed {
        error: CameraError,
        stream_ended: bool,
    },
}

/// The calls one vendor's SDK makes for [`ExposureLoop`].
pub(crate) trait SdkExposure {
    /// Most SDKs refuse new settings mid-stream. ToupTek and SVBony have always applied
    /// first and stopped the stream after; kept as found, since only hardware can say
    /// whether stopping first is safe for them.
    const STOP_STREAM_BEFORE_APPLY: bool = true;

    fn info(&self) -> &CameraInfo;

    /// Pushes a config that differs from the last one applied.
    fn apply(&mut self, config: &CaptureConfig) -> CameraResult<()>;

    fn start(&mut self, acquisition: Acquisition) -> CameraResult<()>;

    /// Ends a running stream for good, to reconfigure or to switch to single exposures.
    fn end_stream(&mut self) {
        self.abort(Acquisition::Stream);
    }

    /// Abandons the exposure in flight on cancel or a stall, best effort.
    fn abort(&mut self, acquisition: Acquisition);

    /// Runs before a stalled exposure is aborted, while the SDK still holds its counters.
    fn on_stall(&mut self, _progress: &Progress) {}

    /// Bytes the frame takes — exactly, since `Frame::from_raw` refuses a buffer of any
    /// other length. Asked once the exposure has started: some SDKs only know it then.
    fn frame_len(&mut self, config: &CaptureConfig) -> CameraResult<usize>;

    fn poll(&mut self, progress: &Progress, buffer: &mut [u8]) -> Poll;

    /// After a single exposure delivered its frame.
    fn finish_single(&mut self) -> CameraResult<()> {
        Ok(())
    }
}

/// The exposure state a vendor camera keeps beside its SDK handle.
#[derive(Default)]
pub(crate) struct ExposureLoop {
    cancel_flag: Arc<AtomicBool>,
    last_applied: Option<CaptureConfig>,
    streaming: bool,
    buffers: BufferPool,
}

impl ExposureLoop {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::SeqCst);
    }

    pub fn cancel_token(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel_flag)
    }

    /// Forgets the last config applied, so the next capture pushes it to the camera again.
    pub fn invalidate(&mut self) {
        self.last_applied = None;
    }

    pub fn capture<S: SdkExposure>(
        &mut self,
        sdk: &mut S,
        config: &CaptureConfig,
    ) -> CameraResult<RawFrame> {
        let started = Instant::now();
        config.validate(sdk.info())?;
        self.cancel_flag.store(false, Ordering::SeqCst);

        if config.should_reapply(self.last_applied.as_ref()) {
            let _span = info_span!("configure_camera", sensor_mode = ?config.sensor_mode).entered();
            if S::STOP_STREAM_BEFORE_APPLY {
                self.end_stream(sdk);
            }
            sdk.apply(config)?;
            self.last_applied = Some(config.clone());
            self.end_stream(sdk);
        }

        let _span = info_span!("read_frame").entered();
        let acquisition = if config.is_continuous() {
            Acquisition::Stream
        } else {
            Acquisition::Single
        };
        match acquisition {
            Acquisition::Stream if self.streaming => {}
            Acquisition::Stream => {
                sdk.start(Acquisition::Stream)?;
                self.streaming = true;
            }
            Acquisition::Single => {
                self.end_stream(sdk);
                sdk.start(Acquisition::Single)?;
            }
        }

        let len = sdk.frame_len(config)?;
        let mut buffer = self.buffers.get(len);
        let budget = config.stall_budget(len);
        let (width, height) = loop {
            if self.cancel_flag.load(Ordering::SeqCst) {
                return Err(self.give_up(sdk, acquisition, CameraError::Cancelled));
            }
            let progress = Progress {
                acquisition,
                config,
                waited: started.elapsed(),
                budget,
            };
            if progress.waited > budget {
                sdk.on_stall(&progress);
                return Err(self.give_up(sdk, acquisition, CameraError::ExposureTimeout(budget)));
            }
            match sdk.poll(&progress, &mut buffer) {
                Poll::Ready { width, height } => break (width, height),
                Poll::Pending => {}
                Poll::Failed {
                    error,
                    stream_ended,
                } => {
                    if stream_ended {
                        self.streaming = false;
                    }
                    return Err(error);
                }
            }
        };

        if acquisition == Acquisition::Single {
            sdk.finish_single()?;
        }
        Ok(RawFrame {
            data: buffer,
            width,
            height,
            format: config.format,
        })
    }

    fn end_stream<S: SdkExposure>(&mut self, sdk: &mut S) {
        if self.streaming {
            sdk.end_stream();
            self.streaming = false;
        }
    }

    fn give_up<S: SdkExposure>(
        &mut self,
        sdk: &mut S,
        acquisition: Acquisition,
        error: CameraError,
    ) -> CameraError {
        sdk.abort(acquisition);
        if acquisition == Acquisition::Stream {
            self.streaming = false;
        }
        error
    }
}

#[cfg(test)]
#[path = "exposure_tests.rs"]
mod tests;
