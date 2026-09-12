//! Startup system report: build, host, CPU, memory and process facts, logged once when the
//! server starts.
//!
//! Field logs arrive without the machine. The 2026-09-07 Orange Pi board resets had to be
//! inferred from PID numbers and a clock stepping backwards; boot time and uptime answer
//! that directly, and the build line says which binary produced the log. Self-contained
//! (std, `sysinfo`, `fs4`, `chrono`, `tokio`) so it compiles for every release target —
//! what only the application knows arrives in [`AppContext`].

mod cpu;
#[cfg(target_os = "linux")]
mod linux;

use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sysinfo::{CpuRefreshKind, MemoryRefreshKind, Motherboard, Product, RefreshKind, System};
use tracing::{info, warn};

/// Collection takes milliseconds, but `statvfs` on a hung network mount never returns and
/// the server must not wait on it.
const COLLECT_TIMEOUT: Duration = Duration::from_secs(5);

const MIB: u64 = 1024 * 1024;

/// Variables that change behaviour, logged with their values.
const ENV_OVERRIDES: &[&str] = &[
    "RUST_LOG",
    "RAYON_NUM_THREADS",
    "TOKIO_WORKER_THREADS",
    "NIGHT_AMPLIFIER_STATIC_DIR",
    "NIGHT_AMPLIFIER_SIM_STALL_EVERY",
    "NIGHT_AMPLIFIER_SIM_STALL_RUN",
    "OTEL_ENABLED",
    "OTEL_SERVICE_NAME",
];

/// Logged as `<set>` only: an endpoint URL can carry credentials.
const ENV_PRESENCE_ONLY: &[&str] = &["OTEL_EXPORTER_OTLP_ENDPOINT"];

const CARGO_FEATURES: &[(&str, bool)] = &[
    ("playerone", cfg!(feature = "playerone")),
    ("zwo", cfg!(feature = "zwo")),
    ("qhy", cfg!(feature = "qhy")),
    ("touptek", cfg!(feature = "touptek")),
    ("svbony", cfg!(feature = "svbony")),
    ("indi", cfg!(feature = "indi")),
    ("telemetry", cfg!(feature = "telemetry")),
    ("bundled-cfitsio", cfg!(feature = "bundled-cfitsio")),
];

/// Facts only the application knows, gathered by `app::run`.
#[derive(Debug, Clone)]
pub struct AppContext {
    pub port: u16,
    pub static_dir: Option<String>,
    pub log_dir: PathBuf,
    pub settings_file: PathBuf,
    pub pro_active: bool,
    pub plugins: Vec<&'static str>,
    pub frame_queue_budget_bytes: usize,
}

/// Collect off the async runtime and log, giving up after [`COLLECT_TIMEOUT`]. Never fails the
/// caller: a missing report costs an investigation some facts, not the observer a session.
pub async fn log_startup_report(app: AppContext) {
    let collection = tokio::task::spawn_blocking(move || SystemReport::collect(app));
    match tokio::time::timeout(COLLECT_TIMEOUT, collection).await {
        Ok(Ok(report)) => report.log(),
        Ok(Err(error)) => warn!(%error, "System report collection failed"),
        Err(_) => warn!(
            timeout_s = COLLECT_TIMEOUT.as_secs(),
            "System report timed out"
        ),
    }
}

#[derive(Debug)]
pub struct SystemReport {
    app: AppContext,
    host: HostFacts,
    cpu: cpu::CpuFacts,
    memory: MemoryFacts,
    process: ProcessFacts,
    #[cfg(target_os = "linux")]
    linux: linux::LinuxFacts,
}

impl SystemReport {
    pub fn collect(app: AppContext) -> Self {
        let system = System::new_with_specifics(
            RefreshKind::nothing()
                .with_memory(MemoryRefreshKind::everything())
                .with_cpu(CpuRefreshKind::nothing().with_frequency()),
        );
        Self {
            host: HostFacts::collect(),
            cpu: cpu::CpuFacts::collect(&system),
            memory: MemoryFacts::collect(&system, app.frame_queue_budget_bytes),
            process: ProcessFacts::collect(&app),
            #[cfg(target_os = "linux")]
            linux: linux::LinuxFacts::collect(),
            app,
        }
    }

    pub fn log(&self) {
        log_build(&self.app);
        self.host.log();
        self.cpu.log();
        self.memory.log();
        self.process.log(&self.app);
        #[cfg(target_os = "linux")]
        self.linux.log();
    }
}

fn log_build(app: &AppContext) {
    let features: Vec<&str> = CARGO_FEATURES
        .iter()
        .filter(|(_, enabled)| *enabled)
        .map(|(name, _)| *name)
        .collect();
    info!(
        version = env!("CARGO_PKG_VERSION"),
        git = option_env!("NIGHT_AMPLIFIER_GIT_DESCRIBE").unwrap_or("unknown"),
        target = option_env!("NIGHT_AMPLIFIER_TARGET").unwrap_or("unknown"),
        target_cpu = option_env!("NIGHT_AMPLIFIER_TARGET_CPU").unwrap_or("unknown"),
        rustc = option_env!("NIGHT_AMPLIFIER_RUSTC_VERSION").unwrap_or("unknown"),
        profile = if cfg!(debug_assertions) { "debug" } else { "release" },
        features = %features.join(","),
        pro_active = app.pro_active,
        plugins = %or_none(&app.plugins.join(",")),
        "System report: build"
    );
}

#[derive(Debug)]
struct HostFacts {
    os: Option<String>,
    kernel: String,
    distribution: String,
    host_name: Option<String>,
    os_arch: String,
    product: Option<String>,
    motherboard: Option<String>,
    uptime_s: u64,
    boot_time_unix_s: u64,
}

impl HostFacts {
    fn collect() -> Self {
        Self {
            os: System::long_os_version(),
            kernel: System::kernel_long_version(),
            distribution: System::distribution_id(),
            host_name: System::host_name(),
            os_arch: System::cpu_arch(),
            product: join_present([Product::vendor_name(), Product::name()]),
            motherboard: Motherboard::new()
                .and_then(|board| join_present([board.vendor_name(), board.name()])),
            uptime_s: System::uptime(),
            boot_time_unix_s: System::boot_time(),
        }
    }

    fn log(&self) {
        info!(
            os = or_unknown(self.os.as_deref()).as_str(),
            kernel = self.kernel.as_str(),
            distribution = self.distribution.as_str(),
            host_name = or_unknown(self.host_name.as_deref()).as_str(),
            os_arch = %self.os_arch,
            product = or_unknown(self.product.as_deref()).as_str(),
            motherboard = or_unknown(self.motherboard.as_deref()).as_str(),
            uptime_s = self.uptime_s,
            boot_time = %utc_rfc3339(self.boot_time_unix_s),
            utc_offset = %chrono::Local::now().offset(),
            "System report: host"
        );
    }
}

#[derive(Debug)]
struct MemoryFacts {
    total: u64,
    available: u64,
    swap_total: u64,
    swap_free: u64,
    cgroup_limit: Option<u64>,
    frame_queue_budget: u64,
}

impl MemoryFacts {
    fn collect(system: &System, frame_queue_budget_bytes: usize) -> Self {
        Self {
            total: system.total_memory(),
            available: system.available_memory(),
            swap_total: system.total_swap(),
            swap_free: system.free_swap(),
            // Most processes sit in a cgroup without a real limit, which reports the host total.
            cgroup_limit: system
                .cgroup_limits()
                .map(|limits| limits.total_memory)
                .filter(|&limit| limit < system.total_memory()),
            frame_queue_budget: frame_queue_budget_bytes as u64,
        }
    }

    fn log(&self) {
        info!(
            total_mib = self.total / MIB,
            available_mib = self.available / MIB,
            swap_total_mib = self.swap_total / MIB,
            swap_free_mib = self.swap_free / MIB,
            cgroup_limit_mib = %self.cgroup_limit.map_or_else(|| "none".to_string(), |b| (b / MIB).to_string()),
            frame_queue_budget_mib = self.frame_queue_budget / MIB,
            "System report: memory"
        );
    }
}

#[derive(Debug)]
struct ProcessFacts {
    pid: u32,
    exe: Option<PathBuf>,
    working_dir: Option<PathBuf>,
    /// `(available, total)` bytes on the working directory's filesystem, where captures go.
    working_dir_space: Option<(u64, u64)>,
    settings_file: PathBuf,
    settings_file_present: bool,
    log_dir: PathBuf,
    env: String,
}

impl ProcessFacts {
    fn collect(app: &AppContext) -> Self {
        let working_dir = std::env::current_dir().ok();
        let resolve = |path: &Path| match &working_dir {
            Some(dir) if path.is_relative() => dir.join(path),
            _ => path.to_path_buf(),
        };
        let settings_file = resolve(&app.settings_file);
        Self {
            pid: std::process::id(),
            exe: std::env::current_exe().ok(),
            working_dir_space: working_dir
                .as_deref()
                .and_then(|dir| fs4::statvfs(dir).ok())
                .map(|stats| (stats.available_space(), stats.total_space())),
            settings_file_present: settings_file.is_file(),
            settings_file,
            log_dir: resolve(&app.log_dir),
            working_dir,
            env: env_overrides(|key| std::env::var(key).ok()),
        }
    }

    fn log(&self, app: &AppContext) {
        let (available, total) = self.working_dir_space.unzip();
        info!(
            pid = self.pid,
            exe = or_unknown(self.exe.as_deref().map(Path::display)).as_str(),
            working_dir = or_unknown(self.working_dir.as_deref().map(Path::display)).as_str(),
            working_dir_available_mib = %or_unknown(available.map(|b| b / MIB)),
            working_dir_total_mib = %or_unknown(total.map(|b| b / MIB)),
            settings_file = self.settings_file.display().to_string().as_str(),
            settings_file_present = self.settings_file_present,
            log_dir = self.log_dir.display().to_string().as_str(),
            port = app.port,
            static_dir = app.static_dir.as_deref().unwrap_or("embedded"),
            env = self.env.as_str(),
            "System report: process"
        );
    }
}

/// `KEY=value` for each override that is set, space-separated, or `none`.
fn env_overrides(lookup: impl Fn(&str) -> Option<String>) -> String {
    let values = ENV_OVERRIDES
        .iter()
        .filter_map(|key| lookup(key).map(|value| format!("{key}={value}")));
    let presence = ENV_PRESENCE_ONLY
        .iter()
        .filter(|key| lookup(key).is_some())
        .map(|key| format!("{key}=<set>"));
    or_none(&values.chain(presence).collect::<Vec<_>>().join(" ")).to_string()
}

fn join_present<const N: usize>(parts: [Option<String>; N]) -> Option<String> {
    let joined = parts
        .into_iter()
        .flatten()
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    (!joined.is_empty()).then_some(joined)
}

fn utc_rfc3339(unix_s: u64) -> String {
    i64::try_from(unix_s)
        .ok()
        .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0))
        .map_or_else(|| "unknown".to_string(), |time| time.to_rfc3339())
}

fn or_unknown<T: Display>(value: Option<T>) -> String {
    value.map_or_else(|| "unknown".to_string(), |value| value.to_string())
}

fn or_none(joined: &str) -> &str {
    if joined.is_empty() {
        "none"
    } else {
        joined
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> AppContext {
        AppContext {
            port: 9955,
            static_dir: None,
            log_dir: PathBuf::from("logs"),
            settings_file: PathBuf::from("settings.json"),
            pro_active: false,
            plugins: Vec::new(),
            frame_queue_budget_bytes: 64 * MIB as usize,
        }
    }

    #[test]
    fn env_overrides_log_values_but_only_the_presence_of_endpoints() {
        let lookup = |key: &str| match key {
            "RUST_LOG" => Some("debug".to_string()),
            "OTEL_EXPORTER_OTLP_ENDPOINT" => Some("https://user:secret@collector".to_string()),
            _ => None,
        };

        let env = env_overrides(lookup);

        assert_eq!(env, "RUST_LOG=debug OTEL_EXPORTER_OTLP_ENDPOINT=<set>");
        assert!(!env.contains("secret"));
    }

    #[test]
    fn env_overrides_say_none_when_nothing_is_set() {
        assert_eq!(env_overrides(|_| None), "none");
    }

    #[test]
    fn join_present_skips_missing_and_blank_parts() {
        assert_eq!(
            join_present([
                Some(" Raspberry Pi ".to_string()),
                None,
                Some("5".to_string())
            ]),
            Some("Raspberry Pi 5".to_string())
        );
        assert_eq!(join_present([None, Some("  ".to_string())]), None);
    }

    #[test]
    fn boot_time_formats_as_utc() {
        assert_eq!(utc_rfc3339(1_788_782_400), "2026-09-07T12:00:00+00:00");
        assert_eq!(utc_rfc3339(u64::MAX), "unknown");
    }

    /// Runs every real probe on the test host; `log` must not panic on whatever it found.
    #[test]
    fn a_report_collected_on_this_host_is_plausible() {
        let report = SystemReport::collect(app());

        assert!(report.memory.total > 0);
        assert!(report.memory.available <= report.memory.total);
        assert!(report.cpu.logical >= 1);
        assert!(!report.host.kernel.is_empty());
        assert!(report.process.log_dir.is_absolute());
        assert!(report
            .process
            .working_dir_space
            .is_some_and(|(available, total)| total > 0 && available <= total));
        report.log();
    }

    #[tokio::test]
    async fn the_startup_report_finishes_well_inside_its_timeout() {
        let started = std::time::Instant::now();

        log_startup_report(app()).await;

        assert!(started.elapsed() < COLLECT_TIMEOUT);
    }
}
