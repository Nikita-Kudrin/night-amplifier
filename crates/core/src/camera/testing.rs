//! A programmable [`Camera`] for tests.
//!
//! Server tests need cameras that stall, panic, die mid-session, block in `close()` or
//! count what they were asked to do. [`FakeCamera`] is all of those with a few knobs
//! turned. Behaviour fixed at construction is set with the builder. Behaviour a test
//! changes, or reads back, while the camera is boxed inside the server lives in the
//! shared [`CameraControls`]. [`FakeCatalog`] puts one on the bus the server connects
//! through. Other crates get this module with the `test-support` feature.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{
    mark_device_lost, Camera, CameraEntry, CameraError, CameraInfo, CameraResult, CameraStatus,
    CaptureConfig, DeviceCatalog, DeviceIdentity, GainPresets, ImageFormat, OpenedCamera,
    RawFrame, SensorType,
};

/// One scripted exposure, played in order before the camera falls back to frames.
pub enum Exposure {
    Frame,
    /// The SDK gave up waiting for the frame.
    Stall,
    /// An exposure that only ends once the flag is raised.
    BlockUntil(Arc<AtomicBool>),
}

/// Knobs a test turns while the camera runs, and what the camera was asked to do.
///
/// Shared, so a catalog can hand the same controls to every camera it opens.
#[derive(Default)]
pub struct CameraControls {
    /// How long `status()` takes.
    pub status_delay_ms: AtomicU64,
    /// How long an exposure takes. Unlike [`FakeCamera::stuck_for`], `cancel()` cuts it
    /// short, but only for a camera built [`FakeCamera::cancellable`].
    pub exposure_ms: AtomicU64,
    /// The next this-many `status()` calls panic. `usize::MAX` panics for good.
    pub status_panics: AtomicUsize,
    /// The next this-many exposures time out.
    pub stalls: AtomicUsize,
    /// The next this-many exposures fail.
    pub failures: AtomicUsize,
    /// The next this-many exposures report the camera disconnected.
    pub lost: AtomicUsize,
    /// While set, every call answers with the device-loss code a shim produces after a
    /// USB reset.
    pub dead: AtomicBool,
    /// Runs inside the next `set_dew_heater` call, the last step of installing a camera.
    pub on_dew_heater: Mutex<Option<Box<dyn FnOnce() + Send>>>,

    pub frames: AtomicUsize,
    pub closes: AtomicUsize,
    pub drops: AtomicUsize,
    pub status_reads: AtomicUsize,
    /// The cooler setpoint each exposure was asked to hold.
    pub setpoints: Mutex<Vec<Option<f64>>>,
    /// Every `(enabled, power)` the dew heater was driven with.
    pub dew_heater: Mutex<Vec<(bool, i32)>>,
}

impl CameraControls {
    pub fn panic_on_status(&self, panics: bool) {
        let count = if panics { usize::MAX } else { 0 };
        self.status_panics.store(count, Ordering::SeqCst);
    }

    fn check_alive(&self) -> CameraResult<()> {
        if self.dead.load(Ordering::SeqCst) {
            return Err(CameraError::CoolingFailed(mark_device_lost(
                "POASetConfig failed: POA_ERROR_NOT_OPENED",
            )));
        }
        Ok(())
    }
}

/// Spends one unit of a "the next N calls" counter; `usize::MAX` never runs out.
fn take_one(counter: &AtomicUsize) -> bool {
    counter
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| match left {
            0 => None,
            usize::MAX => Some(usize::MAX),
            left => Some(left - 1),
        })
        .is_ok()
}

/// A 32x24 mono camera that delivers a frame of 7s on every exposure until told otherwise.
pub struct FakeCamera {
    info: CameraInfo,
    provider: &'static str,
    controls: Arc<CameraControls>,
    cancel_flag: Arc<AtomicBool>,
    cancellable: bool,
    stuck_for: Duration,
    temperature_c: f64,
    script: VecDeque<Exposure>,
    /// Frames left once the script is spent; `None` delivers frames forever.
    frames_after_script: Option<usize>,
    when_spent: Option<Box<dyn Fn() + Send + Sync>>,
    close_blocks_until: Option<Arc<AtomicBool>>,
    during_exposure: Option<Box<dyn FnMut() + Send>>,
    on_frame: Option<Box<dyn FnMut(usize) + Send>>,
}

impl FakeCamera {
    pub fn new(name: &str) -> Self {
        Self::sharing(name, Arc::default())
    }

    /// A camera reporting to (and steered by) controls the test already holds.
    pub fn sharing(name: &str, controls: Arc<CameraControls>) -> Self {
        Self {
            info: CameraInfo {
                name: name.to_string(),
                max_width: 32,
                max_height: 24,
                sensor_type: SensorType::Mono,
                supported_formats: vec![ImageFormat::Raw8, ImageFormat::Raw16],
                ..Default::default()
            },
            provider: "Mock",
            controls,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            cancellable: false,
            stuck_for: Duration::ZERO,
            temperature_c: 0.0,
            script: VecDeque::new(),
            frames_after_script: None,
            when_spent: None,
            close_blocks_until: None,
            during_exposure: None,
            on_frame: None,
        }
    }

    pub fn controls(&self) -> Arc<CameraControls> {
        Arc::clone(&self.controls)
    }

    pub fn with_info(mut self, edit: impl FnOnce(&mut CameraInfo)) -> Self {
        edit(&mut self.info);
        self
    }

    pub fn sized(self, width: u32, height: u32) -> Self {
        self.with_info(|info| {
            info.max_width = width;
            info.max_height = height;
        })
    }

    /// A TEC that reaches -40..30 °C.
    pub fn cooled(self) -> Self {
        self.with_info(|info| {
            info.has_cooler = true;
            info.min_temp_c = Some(-40.0);
            info.max_temp_c = Some(30.0);
        })
    }

    /// How long `status()` takes — a slow USB answer.
    pub fn status_delay(self, delay: Duration) -> Self {
        self.controls.status_delay_ms.store(delay.as_millis() as u64, Ordering::SeqCst);
        self
    }

    pub fn provider(mut self, provider: &'static str) -> Self {
        self.provider = provider;
        self
    }

    /// The sensor temperature `status()` reports.
    pub fn reporting_temperature(mut self, temperature_c: f64) -> Self {
        self.temperature_c = temperature_c;
        self
    }

    /// Every exposure starts with this long a wait that `cancel()` cannot cut short —
    /// an SDK call that does not return.
    pub fn stuck_for(mut self, wait: Duration) -> Self {
        self.stuck_for = wait;
        self
    }

    /// `cancel()` ends the exposure in flight, or the next one if none is.
    pub fn cancellable(mut self) -> Self {
        self.cancellable = true;
        self
    }

    pub fn scripted(mut self, script: impl IntoIterator<Item = Exposure>) -> Self {
        self.script = script.into_iter().collect();
        self
    }

    /// Once the script is spent, `count` more frames; after them every exposure calls
    /// `when_spent` and reports itself cancelled, so a loop driving the camera ends.
    pub fn then_frames(mut self, count: usize, when_spent: impl Fn() + Send + Sync + 'static) -> Self {
        self.frames_after_script = Some(count);
        self.when_spent = Some(Box::new(when_spent));
        self
    }

    pub fn close_blocks_until(mut self, release: Arc<AtomicBool>) -> Self {
        self.close_blocks_until = Some(release);
        self
    }

    /// Runs inside every exposure, after the stuck wait: an observer acting mid-exposure.
    pub fn during_exposure(mut self, hook: impl FnMut() + Send + 'static) -> Self {
        self.during_exposure = Some(Box::new(hook));
        self
    }

    /// Runs with the running frame count each time a frame is delivered.
    pub fn on_frame(mut self, hook: impl FnMut(usize) + Send + 'static) -> Self {
        self.on_frame = Some(Box::new(hook));
        self
    }

    fn frame(&mut self) -> CameraResult<RawFrame> {
        let delivered = self.controls.frames.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(hook) = self.on_frame.as_mut() {
            hook(delivered);
        }
        let pixels = (self.info.max_width * self.info.max_height) as usize;
        Ok(RawFrame {
            data: vec![7u8; pixels].into(),
            width: self.info.max_width,
            height: self.info.max_height,
            format: ImageFormat::Raw8,
        })
    }

    fn expose(&self) -> CameraResult<()> {
        let exposure = Duration::from_millis(self.controls.exposure_ms.load(Ordering::SeqCst));
        let started = Instant::now();
        while started.elapsed() < exposure && !self.cancelled() {
            std::thread::sleep(Duration::from_millis(10));
        }
        if self.cancellable && self.cancel_flag.swap(false, Ordering::SeqCst) {
            return Err(CameraError::Cancelled);
        }
        Ok(())
    }

    fn cancelled(&self) -> bool {
        self.cancellable && self.cancel_flag.load(Ordering::SeqCst)
    }

    fn scripted_failure(&self) -> CameraResult<()> {
        let controls = &self.controls;
        if take_one(&controls.stalls) {
            return Err(CameraError::ExposureTimeout(Duration::from_millis(20)));
        }
        if take_one(&controls.failures) {
            return Err(CameraError::ExposureFailed("scripted failure".to_string()));
        }
        if take_one(&controls.lost) {
            return Err(CameraError::Disconnected);
        }
        Ok(())
    }

    fn after_script(&mut self) -> CameraResult<RawFrame> {
        match self.frames_after_script {
            None => self.frame(),
            Some(0) => {
                if let Some(spent) = &self.when_spent {
                    spent();
                }
                std::thread::sleep(Duration::from_millis(5));
                Err(CameraError::Cancelled)
            }
            Some(left) => {
                self.frames_after_script = Some(left - 1);
                self.frame()
            }
        }
    }
}

impl Drop for FakeCamera {
    fn drop(&mut self) {
        self.controls.drops.fetch_add(1, Ordering::SeqCst);
    }
}

impl Camera for FakeCamera {
    fn info(&self) -> &CameraInfo {
        &self.info
    }

    fn gain_presets(&self) -> CameraResult<GainPresets> {
        Ok(GainPresets::default())
    }

    fn status(&self) -> CameraResult<CameraStatus> {
        let controls = &self.controls;
        controls.status_reads.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(controls.status_delay_ms.load(Ordering::SeqCst)));
        if take_one(&controls.status_panics) {
            panic!("scripted panic in status()");
        }
        controls.check_alive()?;
        Ok(CameraStatus {
            temperature_c: self.temperature_c,
            ..Default::default()
        })
    }

    fn set_target_temperature(&mut self, _temp_c: f64) -> CameraResult<()> {
        self.controls.check_alive()
    }

    fn set_cooler(&mut self, _enabled: bool) -> CameraResult<()> {
        self.controls.check_alive()
    }

    fn set_dew_heater(&mut self, enabled: bool, power: i32) -> CameraResult<()> {
        self.controls.check_alive()?;
        self.controls.dew_heater.lock().unwrap().push((enabled, power));
        let hook = self.controls.on_dew_heater.lock().unwrap().take();
        if let Some(hook) = hook {
            hook();
        }
        Ok(())
    }

    fn capture(&mut self, config: &CaptureConfig) -> CameraResult<RawFrame> {
        self.controls.setpoints.lock().unwrap().push(config.target_temp_c);
        self.controls.check_alive()?;
        if !self.stuck_for.is_zero() {
            std::thread::sleep(self.stuck_for);
        }
        if let Some(hook) = self.during_exposure.as_mut() {
            hook();
        }
        self.expose()?;
        self.scripted_failure()?;
        match self.script.pop_front() {
            Some(Exposure::Frame) => self.frame(),
            Some(Exposure::Stall) => Err(CameraError::ExposureTimeout(Duration::from_millis(1))),
            Some(Exposure::BlockUntil(release)) => {
                while !release.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                self.frame()
            }
            None => self.after_script(),
        }
    }

    fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::SeqCst);
    }

    fn cancel_token(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel_flag)
    }

    fn close(&mut self) -> CameraResult<()> {
        self.controls.closes.fetch_add(1, Ordering::SeqCst);
        if let Some(release) = &self.close_blocks_until {
            while !release.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        Ok(())
    }

    fn provider_name(&self) -> &'static str {
        self.provider
    }
}

/// A bus holding one camera, under the provider [`FakeCatalog::PROVIDER`]: each `open`
/// builds it afresh, so a reconnect gets a new handle the way a real one does.
pub struct FakeCatalog {
    info: CameraInfo,
    make: Box<dyn Fn() -> FakeCamera + Send + Sync>,
}

impl FakeCatalog {
    pub const PROVIDER: &'static str = "Fake";

    pub fn with(make: impl Fn() -> FakeCamera + Send + Sync + 'static) -> Self {
        Self {
            info: make().info.clone(),
            make: Box::new(make),
        }
    }

    /// The id the server lists and connects the camera under.
    pub fn camera_id(&self) -> String {
        super::identity::camera_id(Self::PROVIDER, 0, self.info.serial.as_deref())
    }

    fn check(provider: &str) -> CameraResult<()> {
        if provider.eq_ignore_ascii_case(Self::PROVIDER) {
            return Ok(());
        }
        Err(CameraError::ProviderNotFound(provider.to_string()))
    }
}

impl DeviceCatalog for FakeCatalog {
    fn provider_names(&self, _use_simulated: bool) -> Vec<String> {
        vec![Self::PROVIDER.to_string()]
    }

    fn list(&self, provider: &str, _use_simulated: bool) -> CameraResult<Vec<CameraEntry>> {
        Self::check(provider)?;
        Ok(vec![CameraEntry {
            provider: Self::PROVIDER.to_string(),
            index: 0,
            info: self.info.clone(),
        }])
    }

    fn identities(&self, provider: &str, _use_simulated: bool) -> CameraResult<Vec<DeviceIdentity>> {
        Self::check(provider)?;
        Ok(vec![DeviceIdentity::of(&self.info)])
    }

    fn open(&self, provider: &str, index: usize, _use_simulated: bool) -> CameraResult<OpenedCamera> {
        Self::check(provider)?;
        if index != 0 {
            return Err(CameraError::InvalidCameraIndex { index, count: 1 });
        }
        Ok(OpenedCamera {
            camera: Box::new((self.make)().provider(Self::PROVIDER)),
            provider: Self::PROVIDER.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture(camera: &mut FakeCamera) -> CameraResult<RawFrame> {
        camera.capture(&CaptureConfig::default())
    }

    #[test]
    fn a_plain_camera_delivers_frames_of_its_own_size() {
        let mut camera = FakeCamera::new("Plain").sized(64, 48);
        let frame = capture(&mut camera).unwrap();
        assert_eq!((frame.width, frame.height), (64, 48));
        assert_eq!(frame.data.len(), 64 * 48);
        assert_eq!(camera.controls().frames.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn the_script_plays_before_the_fallback_frames() {
        let release = Arc::new(AtomicBool::new(true));
        let mut camera = FakeCamera::new("Scripted")
            .scripted([Exposure::Stall, Exposure::BlockUntil(release), Exposure::Frame])
            .then_frames(1, || {});
        assert!(matches!(capture(&mut camera), Err(CameraError::ExposureTimeout(_))));
        assert!(capture(&mut camera).is_ok());
        assert!(capture(&mut camera).is_ok());
        assert!(capture(&mut camera).is_ok(), "the one frame after the script");
        assert!(matches!(capture(&mut camera), Err(CameraError::Cancelled)));
    }

    #[test]
    fn a_spent_camera_says_so_every_time() {
        let spent = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&spent);
        let mut camera = FakeCamera::new("Spent").then_frames(0, move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        assert!(capture(&mut camera).is_err());
        assert!(capture(&mut camera).is_err());
        assert_eq!(spent.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn counted_failures_run_out_and_the_camera_recovers() {
        let mut camera = FakeCamera::new("Flaky");
        let controls = camera.controls();
        controls.stalls.store(1, Ordering::SeqCst);
        controls.failures.store(1, Ordering::SeqCst);
        controls.lost.store(1, Ordering::SeqCst);
        assert!(matches!(capture(&mut camera), Err(CameraError::ExposureTimeout(_))));
        assert!(matches!(capture(&mut camera), Err(CameraError::ExposureFailed(_))));
        assert!(matches!(capture(&mut camera), Err(CameraError::Disconnected)));
        assert!(capture(&mut camera).is_ok());
    }

    #[test]
    fn a_dead_camera_fails_every_call_with_the_device_loss_code() {
        let mut camera = FakeCamera::new("Dead");
        camera.controls().dead.store(true, Ordering::SeqCst);
        let Err(CameraError::CoolingFailed(message)) = camera.status() else {
            panic!("status() must fail as a lost device");
        };
        assert!(crate::camera::is_device_lost_message(&message));
        assert!(camera.set_cooler(true).is_err());
        assert!(camera.set_dew_heater(true, 50).is_err());
        assert!(capture(&mut camera).is_err());
    }

    #[test]
    fn status_panics_are_counted_down() {
        let camera = FakeCamera::new("Panicky");
        camera.controls().status_panics.store(1, Ordering::SeqCst);
        let first = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| camera.status()));
        assert!(first.is_err());
        assert!(camera.status().is_ok(), "one scripted panic, then answers again");

        camera.controls().panic_on_status(true);
        for _ in 0..3 {
            let call = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| camera.status()));
            assert!(call.is_err(), "panics for good until switched off");
        }
    }

    #[test]
    fn only_a_cancellable_camera_honours_cancel() {
        let mut stubborn = FakeCamera::new("Stubborn");
        stubborn.cancel();
        assert!(capture(&mut stubborn).is_ok());

        let mut cancellable = FakeCamera::new("Cancellable").cancellable();
        cancellable.controls().exposure_ms.store(10_000, Ordering::SeqCst);
        let token = cancellable.cancel_token();
        let started = Instant::now();
        let cancel = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            token.store(true, Ordering::SeqCst);
        });
        assert!(matches!(capture(&mut cancellable), Err(CameraError::Cancelled)));
        assert!(started.elapsed() < Duration::from_secs(5), "cancel cut the exposure short");
        cancel.join().unwrap();
    }

    #[test]
    fn what_the_camera_was_asked_is_recorded() {
        let mut camera = FakeCamera::new("Recorder").cooled();
        let controls = camera.controls();
        let ran = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&ran);
        *controls.on_dew_heater.lock().unwrap() = Some(Box::new(move || {
            flag.store(true, Ordering::SeqCst)
        }));

        camera.set_dew_heater(true, 40).unwrap();
        camera.set_dew_heater(false, 0).unwrap();
        let mut config = CaptureConfig::default();
        config.target_temp_c = Some(-10.0);
        camera.capture(&config).unwrap();
        camera.status().unwrap();
        camera.close().unwrap();
        drop(camera);

        assert!(ran.load(Ordering::SeqCst), "the install hook ran");
        assert_eq!(*controls.dew_heater.lock().unwrap(), vec![(true, 40), (false, 0)]);
        assert_eq!(*controls.setpoints.lock().unwrap(), vec![Some(-10.0)]);
        assert_eq!(controls.status_reads.load(Ordering::SeqCst), 1);
        assert_eq!(controls.closes.load(Ordering::SeqCst), 1);
        assert_eq!(controls.drops.load(Ordering::SeqCst), 1);
    }
}
