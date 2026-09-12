//! Linux-only procfs/sysfs facts `sysinfo` does not carry. Every one is optional: a
//! container, a board without the driver or a hardened kernel simply lacks the file.

use std::path::{Path, PathBuf};

use tracing::{info, warn};

use super::or_unknown;

#[derive(Debug, Default)]
pub(super) struct LinuxFacts {
    /// Device-tree model — the SBC's name (`Orange Pi 5 Pro`), where DMI has none.
    pub board_model: Option<String>,
    /// Changes on every boot: two logs with different ids had a reset between them.
    pub boot_id: Option<String>,
    /// Camera SDKs open USB devices as this user; non-root needs the vendor udev rules.
    pub effective_uid: Option<u32>,
    pub cpu_governor: Option<String>,
    /// Kernel default 16; vendor udev rules raise it only when their camera is plugged in.
    pub usbfs_memory_mb: Option<u32>,
    /// `(zone type, °C)` of the CPU sensor — see [`is_cpu_zone`].
    pub cpu_temperature: Option<(String, f32)>,
    /// Raspberry Pi firmware under-voltage alarm; `None` off a Pi.
    pub under_voltage: Option<bool>,
}

impl LinuxFacts {
    pub fn collect() -> Self {
        Self {
            board_model: std::fs::read("/proc/device-tree/model")
                .ok()
                .and_then(|raw| parse_device_tree_model(&raw)),
            boot_id: read_trimmed("/proc/sys/kernel/random/boot_id"),
            effective_uid: read_trimmed("/proc/self/status")
                .and_then(|status| parse_effective_uid(&status)),
            cpu_governor: read_trimmed("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"),
            usbfs_memory_mb: read_trimmed("/sys/module/usbcore/parameters/usbfs_memory_mb")
                .and_then(|mb| mb.parse().ok()),
            cpu_temperature: cpu_temperature(),
            under_voltage: rpi_under_voltage(),
        }
    }

    pub fn log(&self) {
        let (thermal_zone, cpu_temp_c) = self
            .cpu_temperature
            .as_ref()
            .map(|(zone, celsius)| (zone.as_str(), format!("{celsius:.1}")))
            .unzip();
        info!(
            board_model = or_unknown(self.board_model.as_deref()).as_str(),
            boot_id = %or_unknown(self.boot_id.as_deref()),
            effective_uid = %or_unknown(self.effective_uid),
            cpu_governor = %or_unknown(self.cpu_governor.as_deref()),
            usbfs_memory_mb = %or_unknown(self.usbfs_memory_mb),
            cpu_temp_c = %or_unknown(cpu_temp_c),
            thermal_zone = %or_unknown(thermal_zone),
            under_voltage = %or_unknown(self.under_voltage),
            "System report: Linux"
        );
        if self.under_voltage == Some(true) {
            warn!("Raspberry Pi firmware reports under-voltage - the supply can reset the board under camera USB load");
        }
    }
}

fn read_trimmed(path: impl AsRef<Path>) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// The first CPU-named thermal zone, else zone 0. `thermal_zone0` alone misleads: on a
/// laptop it is often `acpitz`, a firmware stub that reads a constant 20 °C.
fn cpu_temperature() -> Option<(String, f32)> {
    let mut zones: Vec<(String, PathBuf)> = std::fs::read_dir("/sys/class/thermal")
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("thermal_zone"))
        })
        .filter_map(|path| Some((read_trimmed(path.join("type"))?, path)))
        .collect();
    zones.sort_by(|a, b| a.1.cmp(&b.1));
    let (zone_type, path) = zones
        .iter()
        .find(|(zone_type, _)| is_cpu_zone(zone_type))
        .or_else(|| zones.first())?;
    let celsius = read_trimmed(path.join("temp")).and_then(|raw| parse_millidegrees(&raw))?;
    Some((zone_type.clone(), celsius))
}

/// `cpu-thermal` (Raspberry Pi), `soc-thermal` (RK3588), `x86_pkg_temp` (Intel).
fn is_cpu_zone(zone_type: &str) -> bool {
    let zone_type = zone_type.to_ascii_lowercase();
    ["cpu", "soc", "x86_pkg_temp"]
        .iter()
        .any(|hint| zone_type.contains(hint))
}

/// The `raspberrypi-hwmon` driver's `rpi_volt` sensor, wherever hwmon numbered it.
fn rpi_under_voltage() -> Option<bool> {
    std::fs::read_dir("/sys/class/hwmon")
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|dir| read_trimmed(dir.join("name")).as_deref() == Some("rpi_volt"))
        .and_then(|dir| read_trimmed(dir.join("in0_lcrit_alarm")))
        .map(|alarm| alarm == "1")
}

/// Device-tree strings are NUL-terminated.
fn parse_device_tree_model(raw: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(raw);
    let model = text.trim_end_matches('\0').trim();
    (!model.is_empty()).then(|| model.to_string())
}

fn parse_millidegrees(raw: &str) -> Option<f32> {
    raw.trim()
        .parse::<i64>()
        .ok()
        .map(|millidegrees| millidegrees as f32 / 1000.0)
}

/// `Uid:` lists real, effective, saved and filesystem ids; the second is the one that counts.
fn parse_effective_uid(status: &str) -> Option<u32> {
    status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_tree_model_drops_its_terminator() {
        assert_eq!(
            parse_device_tree_model(b"Orange Pi 5 Pro\0"),
            Some("Orange Pi 5 Pro".to_string())
        );
        assert_eq!(parse_device_tree_model(b"\0"), None);
    }

    #[test]
    fn thermal_zone_reports_millidegrees() {
        assert_eq!(parse_millidegrees("52312\n"), Some(52.312));
        assert_eq!(parse_millidegrees("-4500"), Some(-4.5));
        assert_eq!(parse_millidegrees("n/a"), None);
    }

    #[test]
    fn cpu_zones_are_recognised_across_boards() {
        assert!(is_cpu_zone("cpu-thermal"));
        assert!(is_cpu_zone("soc-thermal"));
        assert!(is_cpu_zone("x86_pkg_temp"));
        assert!(!is_cpu_zone("acpitz"));
        assert!(!is_cpu_zone("iwlwifi_1"));
    }

    #[test]
    fn effective_uid_is_the_second_column() {
        let status = "Name:\tnight-amplifier\nUid:\t1000\t0\t0\t0\nGid:\t1000\t1000\t1000\t1000\n";

        assert_eq!(parse_effective_uid(status), Some(0));
        assert_eq!(parse_effective_uid("Name:\tx\n"), None);
    }
}
