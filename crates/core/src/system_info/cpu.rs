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

        let missing = missing_features(&self.features).join(",");
        if !missing.is_empty() {
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

/// Features this build was compiled for that the host CPU lacks: the ones that crash it.
pub(super) fn missing_features(features: &[Feature]) -> Vec<&'static str> {
    features
        .iter()
        .filter(|feature| feature.compiled && feature.detected == Some(false))
        .map(|feature| feature.name)
        .collect()
}

/// [`missing_features`] for the running host.
pub fn host_missing_features() -> Vec<&'static str> {
    missing_features(&simd_features())
}

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
    let isa = x86::HostIsa::probe();
    x86::REPORTED
        .iter()
        .map(|&name| Feature {
            name,
            compiled: x86::compiled(name),
            detected: Some(isa.has(name)),
        })
        .collect()
}

/// Reads CPUID directly. `is_x86_feature_detected!` answers `true` at compile time for a
/// feature the binary was built with, without asking the CPU, so it cannot see the one
/// mismatch this report exists to catch.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod x86 {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{__cpuid_count, CpuidResult, _xgetbv};
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{__cpuid_count, CpuidResult, _xgetbv};

    pub(super) const REPORTED: [&str; 21] = [
        "sse4.1", "sse4.2", "popcnt", "avx", "avx2", "fma", "bmi1", "bmi2", "lzcnt", "f16c",
        "movbe", "adx", "aes", "pclmulqdq", "sha", "gfni", "vaes", "vpclmulqdq", "avxvnni",
        "avx512f", "avx512bw",
    ];

    pub(super) fn compiled(name: &str) -> bool {
        macro_rules! compiled {
            ($($feature:tt),*) => {
                match name { $($feature => cfg!(target_feature = $feature),)* _ => false }
            };
        }
        compiled!(
            "sse4.1", "sse4.2", "popcnt", "avx", "avx2", "fma", "bmi1", "bmi2", "lzcnt", "f16c",
            "movbe", "adx", "aes", "pclmulqdq", "sha", "gfni", "vaes", "vpclmulqdq", "avxvnni",
            "avx512f", "avx512bw"
        )
    }

    pub(super) struct HostIsa {
        leaf1_ecx: u32,
        leaf7_ebx: u32,
        leaf7_ecx: u32,
        leaf7_1_eax: u32,
        ext1_ecx: u32,
        /// XCR0: which register state the OS saves. A set CPUID bit with a clear one here
        /// still faults.
        xcr0: u64,
    }

    impl HostIsa {
        pub(super) fn probe() -> Self {
            let max_leaf = __cpuid_count(0, 0).eax;
            let max_ext = __cpuid_count(0x8000_0000, 0).eax;
            let leaf = |leaf: u32, sub: u32, max: u32| {
                if leaf <= max {
                    __cpuid_count(leaf, sub)
                } else {
                    CpuidResult { eax: 0, ebx: 0, ecx: 0, edx: 0 }
                }
            };
            let leaf1 = leaf(1, 0, max_leaf);
            let leaf7 = leaf(7, 0, max_leaf);
            Self {
                leaf1_ecx: leaf1.ecx,
                leaf7_ebx: leaf7.ebx,
                leaf7_ecx: leaf7.ecx,
                leaf7_1_eax: leaf(7, 1, if leaf7.eax >= 1 { max_leaf } else { 0 }).eax,
                ext1_ecx: leaf(0x8000_0001, 0, max_ext).ecx,
                xcr0: if leaf1.ecx & (1 << 27) != 0 { read_xcr0() } else { 0 },
            }
        }

        pub(super) fn has(&self, name: &str) -> bool {
            const SSE_AVX_STATE: u64 = 0b110;
            const AVX512_STATE: u64 = 0b1110_0110;
            let bit = |word: u32, n: u32| word & (1 << n) != 0;
            let avx = self.xcr0 & SSE_AVX_STATE == SSE_AVX_STATE && bit(self.leaf1_ecx, 28);
            let avx512 = avx && self.xcr0 & AVX512_STATE == AVX512_STATE;
            match name {
                "sse4.1" => bit(self.leaf1_ecx, 19),
                "sse4.2" => bit(self.leaf1_ecx, 20),
                "popcnt" => bit(self.leaf1_ecx, 23),
                "pclmulqdq" => bit(self.leaf1_ecx, 1),
                "aes" => bit(self.leaf1_ecx, 25),
                "movbe" => bit(self.leaf1_ecx, 22),
                "lzcnt" => bit(self.ext1_ecx, 5),
                "avx" => avx,
                "fma" => avx && bit(self.leaf1_ecx, 12),
                "f16c" => avx && bit(self.leaf1_ecx, 29),
                "avx2" => avx && bit(self.leaf7_ebx, 5),
                "bmi1" => bit(self.leaf7_ebx, 3),
                "bmi2" => bit(self.leaf7_ebx, 8),
                "adx" => bit(self.leaf7_ebx, 19),
                "sha" => bit(self.leaf7_ebx, 29),
                "gfni" => bit(self.leaf7_ecx, 8),
                "vaes" => avx && bit(self.leaf7_ecx, 9),
                "vpclmulqdq" => avx && bit(self.leaf7_ecx, 10),
                "avxvnni" => avx && bit(self.leaf7_1_eax, 4),
                "avx512f" => avx512 && bit(self.leaf7_ebx, 16),
                "avx512bw" => avx512 && bit(self.leaf7_ebx, 30),
                _ => false,
            }
        }
    }

    #[target_feature(enable = "xsave")]
    unsafe fn xgetbv0() -> u64 {
        _xgetbv(0)
    }

    fn read_xcr0() -> u64 {
        // SAFETY: only called when CPUID reports OSXSAVE, which is what makes XGETBV valid.
        unsafe { xgetbv0() }
    }
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

    /// std's detector is authoritative for a feature the binary was not built with.
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    #[test]
    fn cpuid_probe_agrees_with_std_for_features_not_compiled_in() {
        let isa = x86::HostIsa::probe();
        macro_rules! agree {
            ($($name:tt),*) => {$(
                if !x86::compiled($name) {
                    assert_eq!(isa.has($name), std::arch::is_x86_feature_detected!($name), $name);
                }
            )*};
        }
        agree!(
            "sse4.1", "sse4.2", "popcnt", "avx", "avx2", "fma", "bmi1", "bmi2", "lzcnt", "f16c",
            "movbe", "adx", "aes", "pclmulqdq", "sha", "gfni", "vaes", "vpclmulqdq", "avxvnni",
            "avx512f", "avx512bw"
        );
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
