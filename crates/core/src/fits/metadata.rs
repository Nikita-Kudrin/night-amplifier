/// Metadata to include in FITS headers
#[derive(Debug, Clone, Default)]
pub struct FitsMetadata {
    /// Exposure time in seconds
    pub exposure_s: Option<f64>,
    /// Gain value
    pub gain: Option<i32>,
    /// Offset (black level)
    pub offset: Option<i32>,
    /// Camera name
    pub camera: Option<String>,
    /// Object name being captured
    pub object: Option<String>,
    /// Date/time of observation (ISO 8601 format)
    pub date_obs: Option<String>,
    /// Frame number in sequence
    pub frame_number: Option<u64>,
    /// Number of frames stacked (for stacked images)
    pub stacked_frames: Option<u64>,
    /// Software name
    pub software: Option<String>,
    /// CFA pattern if raw bayer data
    pub cfa_pattern: Option<String>,
    /// Binning factor
    pub binning: Option<u8>,
    /// Sensor temperature in Celsius
    pub temperature: Option<f64>,
    /// Target sensor temperature in Celsius (cooled cameras only)
    pub set_temp_c: Option<f64>,
}

impl FitsMetadata {
    /// Create new metadata with default values
    pub fn new() -> Self {
        Self {
            software: Some("Night Amplifier".to_string()),
            ..Default::default()
        }
    }

    /// Set exposure time in microseconds (converts to seconds for FITS)
    pub fn with_exposure_us(mut self, exposure_us: u64) -> Self {
        self.exposure_s = Some(exposure_us as f64 / 1_000_000.0);
        self
    }

    /// Set gain value
    pub fn with_gain(mut self, gain: i32) -> Self {
        self.gain = Some(gain);
        self
    }

    /// Set offset value
    pub fn with_offset(mut self, offset: i32) -> Self {
        self.offset = Some(offset);
        self
    }

    /// Set camera name
    pub fn with_camera(mut self, camera: impl Into<String>) -> Self {
        self.camera = Some(camera.into());
        self
    }

    /// Set observation date/time
    pub fn with_date_obs(mut self, date_obs: impl Into<String>) -> Self {
        self.date_obs = Some(date_obs.into());
        self
    }

    /// Set frame number
    pub fn with_frame_number(mut self, number: u64) -> Self {
        self.frame_number = Some(number);
        self
    }

    /// Set number of stacked frames
    pub fn with_stacked_frames(mut self, count: u64) -> Self {
        self.stacked_frames = Some(count);
        self
    }

    /// Set binning factor
    pub fn with_binning(mut self, bin: u8) -> Self {
        self.binning = Some(bin);
        self
    }

    /// Set sensor temperature in Celsius
    pub fn with_temperature(mut self, temp_c: f64) -> Self {
        self.temperature = Some(temp_c);
        self
    }

    /// Set target sensor temperature in Celsius (cooled cameras only)
    pub fn with_set_temp(mut self, set_temp_c: f64) -> Self {
        self.set_temp_c = Some(set_temp_c);
        self
    }
}
