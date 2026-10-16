//! Refuses to start, with a readable message, on a CPU older than the build.
//!
//! A build made with `-C target-cpu=native` (the `build-dist.sh` default) lets LLVM use any
//! instruction its machine has, GFNI popcounts included, and on a CPU without them the
//! process dies with SIGILL before the first log line. Nothing is missing from the system,
//! so the message names the CPU features and the generic build instead of a dependency.
//! Runs from `.init_array`, ahead of `main`: `#[tokio::main]` has built its runtime by the
//! time a first statement could check.

use super::cpu::host_missing_features;

const RELEASES: &str = "https://github.com/Nikita-Kudrin/night-amplifier/releases";

/// Exits with status 1 and the explanation when the host lacks a feature the build uses.
pub fn exit_if_cpu_unsupported() {
    let missing = host_missing_features();
    if missing.is_empty() {
        return;
    }
    eprintln!("{}", refusal_message(&missing, std::env::consts::ARCH));
    std::process::exit(1);
}

pub(super) fn refusal_message(missing: &[&str], arch: &str) -> String {
    let (generic, rebuild) = match arch {
        "x86_64" => (
            "the x64 build (made for x86-64-v3: any CPU with AVX2)",
            "./scripts/build-dist.sh --target-cpu x86-64-v3",
        ),
        "aarch64" => (
            "the arm64 generic build",
            "./scripts/build-dist.sh --target-cpu generic",
        ),
        _ => ("the generic build for your platform", "./scripts/build-dist.sh --target-cpu generic"),
    };
    format!(
        "Night Amplifier cannot start: this build was compiled for a newer CPU than this computer's.\n\
         CPU features it needs and this computer lacks: {}\n\
         Nothing needs to be installed. Use a build for an older CPU instead:\n\
         \x20 - download {generic} from {RELEASES}\n\
         \x20 - or build it yourself: {rebuild}\n\
         See \"System dependencies\" in the manual.",
        missing.join(", ")
    )
}

/// Runs [`exit_if_cpu_unsupported`] before `main`. Invoke once at module level in each binary;
/// it lives in the binary, not here, because the linker drops a library object that only
/// holds a constructor.
#[macro_export]
macro_rules! refuse_unsupported_cpu_before_main {
    () => {
        #[used]
        #[cfg_attr(
            any(target_os = "linux", target_os = "android", target_os = "freebsd"),
            link_section = ".init_array"
        )]
        #[cfg_attr(target_vendor = "apple", link_section = "__DATA,__mod_init_func")]
        #[cfg_attr(windows, link_section = ".CRT$XCU")]
        static REFUSE_UNSUPPORTED_CPU: extern "C" fn() = {
            extern "C" fn refuse() {
                $crate::system_info::cpu_guard::exit_if_cpu_unsupported();
            }
            refuse
        };
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system_info::cpu::{missing_features, Feature};

    fn feature(name: &'static str, compiled: bool, detected: Option<bool>) -> Feature {
        Feature { name, compiled, detected }
    }

    #[test]
    fn only_compiled_features_the_host_lacks_are_missing() {
        let features = [
            feature("avx2", true, Some(true)),
            feature("gfni", true, Some(false)),
            feature("avx512f", false, Some(false)),
            feature("neon", true, None),
        ];

        assert_eq!(missing_features(&features), ["gfni"]);
    }

    #[test]
    fn message_names_the_features_and_the_way_out() {
        let message = refusal_message(&["gfni", "vaes"], "x86_64");

        assert!(message.contains("gfni, vaes"));
        assert!(message.contains("--target-cpu x86-64-v3"));
        assert!(message.contains(RELEASES));
        assert!(message.contains("Nothing needs to be installed"));
    }

    #[test]
    fn this_host_supports_the_test_build() {
        assert!(host_missing_features().is_empty());
    }
}
