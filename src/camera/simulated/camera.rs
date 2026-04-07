//! Simulated camera implementation

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tracing::{debug, info};

use crate::camera::error::{CameraError, CameraResult};
use crate::camera::traits::Camera;
use crate::camera::types::{
    CameraInfo, CameraStatus, CaptureConfig, GainPresets, ImageFormat, SensorType,
};
use crate::debayer::{CfaPattern, DebayerConfig, Debayerer};
use crate::Frame;

use rayon::prelude::*;

use super::loaders::load_image;
use super::probe::probe_image_dimensions;
use super::registry::find_image_files;

const MAX_PRELOAD_IMAGES: usize = 10;

/// Simulated camera that reads images from files
///
/// Uses a sliding window cache of up to 3 frames to avoid re-decoding from
/// disk on every capture while keeping memory usage bounded. Frames are
/// decoded lazily on the first capture and the window advances as frames
/// are consumed.
pub struct SimulatedCamera {
    info: CameraInfo,
    directory: PathBuf,
    files: Vec<PathBuf>,
    current_index: usize,
    /// Ring-buffer holding at most LOOKAHEAD decoded (and debayered) frames.
    /// `cache[i]` corresponds to file index `cache_start + i`.
    cache: Vec<Frame>,
    cache_start: usize,
    debayerer: Option<Debayerer>,
    cancel_flag: Arc<AtomicBool>,
    current_exposure_us: u64,
    current_gain: i32,
    current_offset: i32,
}

impl SimulatedCamera {
    /// Create a new simulated camera from a directory.
    ///
    /// Construction is lightweight — no images are decoded here.
    /// Frames are loaded lazily on the first `capture()` call.
    pub fn new(directory: PathBuf) -> CameraResult<Self> {
        if !directory.exists() {
            return Err(CameraError::OpenFailed(format!(
                "Directory does not exist: {}",
                directory.display()
            )));
        }

        let files = find_image_files(&directory);
        if files.is_empty() {
            return Err(CameraError::OpenFailed(format!(
                "No image files found in: {}",
                directory.display()
            )));
        }

        // Probe the first file to get dimensions
        let (width, height, sensor_type, bayer_pattern) = probe_image_dimensions(&files[0])?;

        // Extract directory name for camera name
        let dir_name = directory
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        let info = create_camera_info(
            dir_name,
            files.len(),
            width,
            height,
            sensor_type,
            bayer_pattern,
        );

        let debayerer = if sensor_type == SensorType::Color {
            bayer_pattern.map(|p| Debayerer::new(DebayerConfig::new(p)))
        } else {
            None
        };

        info!(
            directory = %directory.display(),
            file_count = files.len(),
            width = width,
            height = height,
            "Simulated camera opened"
        );

        Ok(Self {
            info,
            directory,
            files,
            current_index: 0,
            cache: Vec::with_capacity(MAX_PRELOAD_IMAGES),
            cache_start: 0,
            debayerer,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            current_exposure_us: 1_000_000,
            current_gain: 0,
            current_offset: 10,
        })
    }

    /// Decode a single file and apply debayering if needed.
    fn decode_frame(&self, file_index: usize) -> CameraResult<Frame> {
        let path = &self.files[file_index];
        let frame = load_image(path)?;

        if let Some(ref deb) = self.debayerer {
            if frame.channels() == 1 {
                return deb.debayer(&frame).map_err(|e| {
                    CameraError::ImageReadFailed(format!("Debayering failed: {}", e))
                });
            }
        }

        Ok(frame)
    }

    /// Ensure the sliding window cache covers `current_index` and up to
    /// `lookahead` frames ahead. Only decodes frames not already cached.
    fn fill_cache(&mut self, lookahead: usize) -> CameraResult<()> {
        let file_count = self.files.len();
        let needed_start = self.current_index;

        // If the window has drifted past our cache, reset
        if self.cache.is_empty()
            || needed_start < self.cache_start
            || needed_start >= self.cache_start + self.cache.len()
        {
            let start = Instant::now();
            self.cache_start = needed_start;

            let count = lookahead.min(file_count);
            // Parallel decode all needed frames
            self.cache = (0..count)
                .into_par_iter()
                .map(|i| {
                    let file_idx = (needed_start + i) % file_count;
                    self.decode_frame(file_idx)
                })
                .collect::<Result<Vec<_>, _>>()?;

            debug!(
                cache_start = self.cache_start,
                cache_len = self.cache.len(),
                elapsed_ms = start.elapsed().as_millis() as u64,
                "Cache initialized (parallel)"
            );
            return Ok(());
        }

        // Slide: drop frames before current_index, append new ones ahead
        let drop_count = needed_start - self.cache_start;
        if drop_count > 0 {
            let start = Instant::now();
            self.cache.drain(..drop_count);
            self.cache_start = needed_start;

            // Fill up to lookahead
            let current_len = self.cache.len();
            let target_len = lookahead.min(file_count);
            if current_len < target_len {
                let mut new_frames = (0..(target_len - current_len))
                    .into_par_iter()
                    .map(|i| {
                        let file_idx = (self.cache_start + current_len + i) % file_count;
                        self.decode_frame(file_idx)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                self.cache.append(&mut new_frames);
            }

            debug!(
                cache_start = self.cache_start,
                cache_len = self.cache.len(),
                elapsed_ms = start.elapsed().as_millis() as u64,
                "Cache advanced (parallel)"
            );
        }

        Ok(())
    }

    /// Return a clone of the current frame and advance to the next.
    fn load_current_frame(&mut self, lookahead: usize) -> CameraResult<Frame> {
        if self.files.is_empty() {
            return Err(CameraError::ImageReadFailed(
                "No files available".to_string(),
            ));
        }

        self.fill_cache(lookahead)?;

        let cache_offset = self.current_index - self.cache_start;
        let frame = self.cache[cache_offset].clone();

        debug!(
            index = self.current_index,
            total = self.files.len(),
            "Returning cached simulated frame"
        );

        self.current_index = (self.current_index + 1) % self.files.len();

        Ok(frame)
    }
}

impl Camera for SimulatedCamera {
    fn info(&self) -> &CameraInfo {
        &self.info
    }

    fn gain_presets(&self) -> CameraResult<GainPresets> {
        Ok(GainPresets {
            highest_dr: 0,
            hcg: 120,
            unity: 100,
            lowest_rn: 300,
            offset_highest_dr: 10,
            offset_hcg: 30,
            offset_unity: 20,
            offset_lowest_rn: 50,
        })
    }

    fn status(&self) -> CameraResult<CameraStatus> {
        Ok(CameraStatus {
            temperature_c: 20.0, // Room temperature
            cooler_power: None,
            cooler_on: false,
            is_exposing: false,
            current_gain: self.current_gain,
            current_offset: self.current_offset,
            current_exposure_us: self.current_exposure_us,
        })
    }

    fn set_target_temperature(&mut self, _temp_c: f64) -> CameraResult<()> {
        Err(CameraError::ParameterNotSupported("cooler".to_string()))
    }

    fn set_cooler(&mut self, _enabled: bool) -> CameraResult<()> {
        Err(CameraError::ParameterNotSupported("cooler".to_string()))
    }

    fn capture(&mut self, config: &CaptureConfig) -> CameraResult<Frame> {
        // Store current settings
        self.current_exposure_us = config.exposure_us;
        self.current_gain = config.gain;
        self.current_offset = config.offset;

        self.load_current_frame(config.simulated_preload_images)
    }

    fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::SeqCst);
    }

    fn cancel_token(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel_flag)
    }

    fn close(&mut self) -> CameraResult<()> {
        info!(
            directory = %self.directory.display(),
            "Simulated camera closed"
        );
        Ok(())
    }

    fn provider_name(&self) -> &'static str {
        "Simulator"
    }
}

pub fn create_camera_info(
    dir_name: &str,
    file_count: usize,
    width: u32,
    height: u32,
    sensor_type: SensorType,
    bayer_pattern: Option<crate::CfaPattern>,
) -> CameraInfo {
    CameraInfo {
        name: format!("Simulator: {} ({} files)", dir_name, file_count),
        id: 0,
        max_width: width,
        max_height: height,
        pixel_size_um: 3.76, // Common sensor pixel size
        sensor_type,
        bayer_pattern,
        has_cooler: false,
        has_shutter: false,
        is_usb3: true,
        bit_depth: 16,
        supported_bins: vec![1],
        supported_formats: vec![ImageFormat::Raw16],
        min_exposure_us: 1,
        max_exposure_us: 3600_000_000,
        min_gain: 0,
        max_gain: 500,
        unity_gain: 100,
        hcg_gain: 120,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_simulated_camera_preloading() {
        let dir = tempdir().unwrap();

        // Minimal valid 1x1 PNG data
        let png_data = [
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x08,
            0xd7, 0x63, 0xf8, 0xff, 0xff, 0x3f, 0x00, 0x05, 0xfe, 0x02, 0xfe, 0xdc, 0x44, 0x74,
            0x8e, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
        ];

        for i in 0..10 {
            let file_path = dir.path().join(format!("frame_{:03}.png", i));
            let mut file = std::fs::File::create(file_path).unwrap();
            file.write_all(&png_data).unwrap();
        }

        let mut camera = SimulatedCamera::new(dir.path().to_path_buf()).unwrap();

        // Initial state: cache should be empty
        assert!(camera.cache.is_empty());

        // First capture should fill cache (lookahead = 5)
        let config = CaptureConfig::default().with_simulated_preload_images(5);
        let _ = camera.capture(&config).unwrap();

        assert_eq!(camera.cache.len(), 5);
        assert_eq!(camera.cache_start, 0);
        assert_eq!(camera.current_index, 1);

        // Next capture should still have 5 in cache, current_index advanced
        // It should have dropped index 0 and loaded index 5
        let _ = camera.capture(&config).unwrap();
        assert_eq!(camera.cache.len(), 5);
        assert_eq!(camera.cache_start, 1);
        assert_eq!(camera.current_index, 2);

        // Advance to near end
        camera.current_index = 8;
        let _ = camera.capture(&config).unwrap();
        assert_eq!(camera.cache_start, 8);
        assert_eq!(camera.cache.len(), 5); // 8, 9, 0, 1, 2 (wrap around)
        assert_eq!(camera.current_index, 9);
    }
}
