//! Which physical camera an id names, independent of where the SDK lists it today.
//!
//! Ids used to be `{provider}_{index}`, a position in the vendor's device list. USB
//! re-enumeration reorders that list, so on 2026-09-07 a guide camera's reconnect
//! reopened index 0 — by then the *imaging* camera — and installed it as the guide.
//! Where the SDK exposes a serial without opening the device, the id carries the serial
//! instead; everywhere else a reopened device is checked against the camera it replaces
//! before it is trusted.

use super::types::CameraInfo;

/// What an id says about finding its device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CameraLocator {
    /// A position in the provider's list. Only as good as the moment it was read.
    Index(usize),
    /// A vendor serial number. Found wherever the device is listed now.
    Serial(String),
}

/// The identifying half of a [`CameraInfo`], cheap enough to enumerate without
/// opening a device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceIdentity {
    pub name: String,
    pub serial: Option<String>,
    /// The SDK's own per-device id (ZWO and Player One `cameraID`), stable for as long
    /// as the device stays plugged in. Not an identity across a dropout — only a way to
    /// recognise a device the *other* role still holds. `None` where a provider's id is
    /// just a list position.
    pub device_id: Option<i32>,
}

impl DeviceIdentity {
    pub fn new(name: impl Into<String>, serial: Option<String>) -> Self {
        Self {
            name: name.into(),
            serial,
            device_id: None,
        }
    }

    pub fn with_device_id(mut self, device_id: i32) -> Self {
        self.device_id = Some(device_id);
        self
    }

    pub fn of(info: &CameraInfo) -> Self {
        Self::new(info.name.clone(), info.serial.clone())
    }

    /// Whether two identities name one physical device, when the SDK says: the serials
    /// if both sides have one, else the SDK device ids if both sides have one. `None`
    /// when only the model name is left, which cannot tell two bodies apart — the caller
    /// falls back to something it knows, such as the device's position.
    pub fn is_same_device(&self, other: &DeviceIdentity) -> Option<bool> {
        if let (Some(a), Some(b)) = (&self.serial, &other.serial) {
            return Some(a == b);
        }
        match (self.device_id, other.device_id) {
            (Some(a), Some(b)) => Some(a == b),
            _ => None,
        }
    }

    /// Whether two identities can be the same device. Serials decide when both sides
    /// have one; otherwise only the model name is left to compare.
    pub fn matches(&self, other: &DeviceIdentity) -> bool {
        match (&self.serial, &other.serial) {
            (Some(a), Some(b)) => a == b,
            _ => self.name == other.name,
        }
    }
}

/// Why an id did not parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraIdError {
    /// No `provider_` prefix.
    Format,
    /// The part after the prefix is neither an index nor a serial.
    Locator,
}

const SERIAL_PREFIX: &str = "sn-";
const HEX_SERIAL_PREFIX: &str = "snx-";

/// Clean a raw vendor serial: NUL padding and whitespace trimmed, and the empty or
/// all-zero placeholders some firmwares report mapped to `None` — two bodies both
/// reporting "0000" would otherwise be treated as one device.
pub fn normalize_serial(raw: &str) -> Option<String> {
    let trimmed = raw.trim_matches(|c: char| c == '\0' || c.is_whitespace());
    if trimmed.is_empty() || trimmed.chars().all(|c| c == '0') {
        return None;
    }
    Some(trimmed.to_string())
}

/// The id a device is published under: its serial when it has one, its index otherwise.
///
/// Serials outside `[A-Za-z0-9-]` are hex-encoded, because ids travel in URL paths.
pub fn camera_id(provider: &str, index: usize, serial: Option<&str>) -> String {
    let provider = provider.to_lowercase();
    let Some(serial) = serial else {
        return format!("{provider}_{index}");
    };
    if serial.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return format!("{provider}_{SERIAL_PREFIX}{serial}");
    }
    let hex: String = serial.bytes().map(|b| format!("{b:02x}")).collect();
    format!("{provider}_{HEX_SERIAL_PREFIX}{hex}")
}

/// Split an id into its provider and locator. Index ids from before serials existed
/// still parse.
pub fn parse_camera_id(camera_id: &str) -> Result<(&str, CameraLocator), CameraIdError> {
    let (provider, rest) = camera_id.split_once('_').ok_or(CameraIdError::Format)?;
    if provider.is_empty() {
        return Err(CameraIdError::Format);
    }
    if let Some(hex) = rest.strip_prefix(HEX_SERIAL_PREFIX) {
        return decode_hex(hex)
            .map(|serial| (provider, CameraLocator::Serial(serial)))
            .ok_or(CameraIdError::Locator);
    }
    if let Some(serial) = rest.strip_prefix(SERIAL_PREFIX) {
        if serial.is_empty() {
            return Err(CameraIdError::Locator);
        }
        return Ok((provider, CameraLocator::Serial(serial.to_string())));
    }
    rest.parse()
        .map(|index| (provider, CameraLocator::Index(index)))
        .map_err(|_| CameraIdError::Locator)
}

fn decode_hex(hex: &str) -> Option<String> {
    if hex.is_empty() || !hex.len().is_multiple_of(2) {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect();
    String::from_utf8(bytes?).ok()
}

/// Where `locator` is listed now.
///
/// A serial that is not listed is `None`, never a fallback to some index: a device that
/// is missing is waited for, not guessed at.
pub fn resolve_index(listed: &[DeviceIdentity], locator: &CameraLocator) -> Option<usize> {
    match locator {
        CameraLocator::Index(index) => (*index < listed.len()).then_some(*index),
        CameraLocator::Serial(serial) => listed
            .iter()
            .position(|identity| identity.serial.as_deref() == Some(serial.as_str())),
    }
}

/// The indices worth reopening to get `expected` back, best first.
///
/// With a serial there is exactly one answer. Without one, every listed device of the
/// same model is a candidate, the index it was last seen at first — except a device
/// that is recognisably the camera holding the other role (same serial, or same SDK
/// device id), which reopening would steal.
pub fn recovery_candidates(
    listed: &[DeviceIdentity],
    expected: &DeviceIdentity,
    last_index: usize,
    other_role: Option<&DeviceIdentity>,
) -> Vec<usize> {
    if let Some(serial) = &expected.serial {
        if listed.iter().any(|identity| identity.serial.is_some()) {
            return resolve_index(listed, &CameraLocator::Serial(serial.clone()))
                .into_iter()
                .collect();
        }
    }

    let held_elsewhere = |identity: &DeviceIdentity| {
        other_role.is_some_and(|other| identity.is_same_device(other) == Some(true))
    };
    let mut candidates: Vec<usize> = listed
        .iter()
        .enumerate()
        .filter(|(_, identity)| identity.name == expected.name && !held_elsewhere(identity))
        .map(|(index, _)| index)
        .collect();
    if let Some(position) = candidates.iter().position(|&index| index == last_index) {
        let last_seen = candidates.remove(position);
        candidates.insert(0, last_seen);
    }
    candidates
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
