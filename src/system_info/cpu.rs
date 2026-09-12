//! CPU facts, and the SIMD this binary was compiled for against what the host has.
//!
//! Release artifacts are built per CPU (`x86-64-v3`, `cortex-a76`, `apple-m1`, `generic`) and
//! `wide` picks its SIMD at compile time, so a mismatch is invisible at runtime: a feature
//! compiled in but absent crashes with SIGILL inside SIMD code, and one present but not
//! compiled is silently lost speed (the generic arm64 artifact on a Pi 5).

use sysinfo::System;
use tracing::{info, warn};

use super::or_unknown;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Feature {
    pub name: &'static str,
    pub compiled: bool,
    /// `None` where std cannot detect features at runtime on this architecture.
    pub detected: Option<bool>,
}

#[derive(Debug)]
pub(super) struct CpuFacts {
    pub cores: String,
    pub vendor: String,
    pub logical: usize,
    pub physical: Option<usize>,
    pub available_parallelism: Option<usize>,
    /// Highest current frequency across cores at startup — low under a `powersave` governor.
    pub frequency_mhz: Option<u64>,
    pub features: Vec<Feature>,
    pub emulation_suspected: bool,
}

impl CpuFacts {
    pub fn collect(system: &System) -> Self {
        let cpus = system.cpus();
        let brands: Vec<&str> = cpus.iter().map(|cpu| cpu.brand()).collect();
        let cores = summarize_brands(&brands);
        Self {
            emulation_suspected: emulation_suspected(std::env::consts::ARCH, &cores),
            cores,
            vendor: cpus
                .first()
                .map_or_else(String::new, |cpu| cpu.vendor_id().to_string()),
            logical: cpus.len(),
            physical: System::physical_core_count(),
            available_parallelism: std::thread::available_parallelism()
                .ok()
                .map(|threads| threads.get()),
            frequency_mhz: cpus
                .iter()
                .map(|cpu| cpu.frequency())
                .max()
                .filter(|&mhz| mhz > 0),
            features: simd_features(),
        }
    }

    pub fn log(&self) {
        info!(
            cores = self.cores.as_str(),
            vendor = self.vendor.as_str(),
            logical_cpus = self.logical,
            physical_cores = %or_unknown(self.physical),
            available_parallelism = %or_unknown(self.available_parallelism),
            frequency_mhz = %or_unknown(self.frequency_mhz),
            simd_compiled = %self.feature_names(|f| f.compiled),
            simd_detected = %self.feature_names(|f| f.detected == Some(true)),
            simd_unused_by_build = %self.feature_names(|f| !f.compiled && f.detected == Some(true)),
            "System report: CPU"
        );

        let missing = self.feature_names(|f| f.compiled && f.detected == Some(false));
        if missing != NONE {
            warn!(
                %missing,
                "This build uses CPU features the host lacks - SIMD code will crash; use the generic build"
            );
        }
        if self.emulation_suspected {
            warn!(
                binary_arch = std::env::consts::ARCH,
                cores = self.cores.as_str(),
                "x86 build on an Arm CPU runs under emulation, several times slower - use the native build"
            );
        }
    }

    fn feature_names(&self, pick: impl Fn(&Feature) -> bool) -> String {
        let names: Vec<&str> = self
            .features
            .iter()
            .filter(|feature| pick(feature))
            .map(|feature| feature.name)
            .collect();
        if names.is_empty() {
            NONE.to_string()
        } else {
            names.join(",")
        }
    }
}

const NONE: &str = "none";

/// `4x Cortex-A55, 4x Cortex-A76` — a hybrid SoC's clusters, in the order the OS lists them.
pub(super) fn summarize_brands(brands: &[&str]) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for brand in brands {
        let brand = match brand.trim() {
            "" => "unknown",
            trimmed => trimmed,
        };
        match counts.iter_mut().find(|(seen, _)| *seen == brand) {
            Some((_, count)) => *count += 1,
            None => counts.push((brand, 1)),
        }
    }
    if counts.is_empty() {
        return "unknown".to_string();
    }
    counts
        .iter()
        .map(|(brand, count)| format!("{count}x {brand}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// An x86 binary on an Arm CPU runs translated (Rosetta 2, Windows' x64 emulation). The OS
/// arch an emulated process can query reports the emulated arch, but the CPU brand string
/// comes from the host. Windows can hand an emulated process a virtual brand instead, so this
/// is a hint, not proof.
pub(super) fn emulation_suspected(binary_arch: &str, cpu_brands: &str) -> bool {
    const ARM_HOST_BRANDS: [&str; 6] = [
        "Apple M",
        "Snapdragon",
        "Microsoft SQ",
        "Ampere",
        "Neoverse",
        "Cortex-A",
    ];
    matches!(binary_arch, "x86" | "x86_64")
        && ARM_HOST_BRANDS
            .iter()
            .any(|brand| cpu_brands.contains(brand))
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
pub(super) fn simd_features() -> Vec<Feature> {
    macro_rules! probe {
        ($($name:tt),* $(,)?) => {
            vec![$(Feature {
                name: $name,
                compiled: cfg!(target_feature = $name),
                detected: Some(std::arch::is_x86_feature_detected!($name)),
            }),*]
        };
    }
    probe![
        "sse4.1", "sse4.2", "popcnt", "avx", "avx2", "fma", "bmi1", "bmi2", "lzcnt", "f16c",
        "movbe", "avx512f", "avx512bw", "avx512vl",
    ]
}

#[cfg(target_arch = "aarch64")]
pub(super) fn simd_features() -> Vec<Feature> {
    macro_rules! probe {
        ($($name:tt),* $(,)?) => {
            vec![$(Feature {
                name: $name,
                compiled: cfg!(target_feature = $name),
                detected: Some(std::arch::is_aarch64_feature_detected!($name)),
            }),*]
        };
    }
    probe!["neon", "crc", "aes", "sha2", "lse", "rdm", "fp16", "dotprod", "i8mm", "sve", "sve2"]
}

/// Runtime detection on 32-bit Arm is unstable in std, so only the build side is known.
#[cfg(target_arch = "arm")]
pub(super) fn simd_features() -> Vec<Feature> {
    vec![Feature {
        name: "neon",
        compiled: cfg!(target_feature = "neon"),
        detected: None,
    }]
}

#[cfg(not(any(
    target_arch = "x86",
    target_arch = "x86_64",
    target_arch = "aarch64",
    target_arch = "arm"
)))]
pub(super) fn simd_features() -> Vec<Feature> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hybrid_clusters_are_counted_per_brand_in_first_seen_order() {
        let brands = [
            "Cortex-A55",
            "Cortex-A55",
            "Cortex-A76",
            "Cortex-A76",
            "Cortex-A55",
        ];

        assert_eq!(summarize_brands(&brands), "3x Cortex-A55, 2x Cortex-A76");
    }

    #[test]
    fn blank_brands_are_reported_as_unknown() {
        assert_eq!(summarize_brands(&["", " "]), "2x unknown");
        assert_eq!(summarize_brands(&[]), "unknown");
    }

    #[test]
    fn an_x86_build_on_an_arm_cpu_is_flagged() {
        assert!(emulation_suspected("x86_64", "10x Apple M1 Pro"));
        assert!(emulation_suspected("x86_64", "8x Snapdragon(R) X Elite"));
        assert!(!emulation_suspected("aarch64", "10x Apple M1 Pro"));
        assert!(!emulation_suspected("x86_64", "32x AMD Ryzen 9 7950X"));
    }

    /// The test binary is running, so whatever it was compiled for must be present — a
    /// `compiled && !detected` here means the probe itself is wrong.
    #[test]
    fn every_feature_this_test_binary_was_compiled_with_is_detected() {
        for feature in simd_features() {
            assert_ne!(
                (feature.compiled, feature.detected),
                (true, Some(false)),
                "{}",
                feature.name
            );
        }
    }

    #[test]
    fn feature_names_say_none_when_nothing_matches() {
        let facts = CpuFacts {
            cores: String::new(),
            vendor: String::new(),
            logical: 1,
            physical: None,
            available_parallelism: None,
            frequency_mhz: None,
            features: vec![Feature {
                name: "avx2",
                compiled: false,
                detected: Some(true),
            }],
            emulation_suspected: false,
        };

        assert_eq!(facts.feature_names(|f| f.compiled), "none");
        assert_eq!(facts.feature_names(|f| f.detected == Some(true)), "avx2");
    }
}
