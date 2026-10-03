//! Where the AI denoiser runs, as data. The hardware benchmark, the backends and the choice
//! live in the Pro repo's `plugins::ai_denoise::compute`; Community carries the report to
//! `GET /api/ai-compute` and the observer's "AI compute" choice to `settings.json`.
//!
//! The report is built for one preference at a time ([`super::ai::compute_report`]), so the
//! plugin keeps no copy of a setting Community owns.

use serde::{Deserialize, Deserializer, Serialize};

/// A kind of unit the network can run on, in the order the benchmark tries them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeRung {
    Npu,
    DiscreteGpu,
    IntegratedGpu,
    Cpu,
}

impl ComputeRung {
    pub const LADDER: [Self; 4] = [Self::Npu, Self::DiscreteGpu, Self::IntegratedGpu, Self::Cpu];

    pub fn label(self) -> &'static str {
        match self {
            Self::Npu => "NPU",
            Self::DiscreteGpu => "Dedicated GPU",
            Self::IntegratedGpu => "Integrated GPU",
            Self::Cpu => "CPU",
        }
    }
}

/// The "AI compute" setting: let the benchmark decide, or force one rung.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AiComputePreference {
    #[default]
    Auto,
    Npu,
    DiscreteGpu,
    IntegratedGpu,
    Cpu,
}

impl AiComputePreference {
    /// The rung this forces, or `None` for Auto.
    pub fn forced(self) -> Option<ComputeRung> {
        match self {
            Self::Auto => None,
            Self::Npu => Some(ComputeRung::Npu),
            Self::DiscreteGpu => Some(ComputeRung::DiscreteGpu),
            Self::IntegratedGpu => Some(ComputeRung::IntegratedGpu),
            Self::Cpu => Some(ComputeRung::Cpu),
        }
    }
}

/// Anything this build does not know — a value from a newer version, a typo, `null` — reads
/// as Auto. One unreadable value would otherwise fail the whole of `settings.json`.
impl<'de> Deserialize<'de> for AiComputePreference {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Option::<serde_json::Value>::deserialize(deserializer)?;
        Ok(match value.as_ref().and_then(|v| v.as_str()) {
            Some("npu") => Self::Npu,
            Some("discrete_gpu") => Self::DiscreteGpu,
            Some("integrated_gpu") => Self::IntegratedGpu,
            Some("cpu") => Self::Cpu,
            _ => Self::Auto,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkState {
    /// No AI denoiser in this build, or no Pro licence.
    Unavailable,
    /// Looking for compute units and a stored result for this machine.
    Checking,
    /// Measuring each unit; the UI blocks until this ends.
    Benchmarking,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkProgress {
    /// Units finished, and how many there are.
    pub done: u32,
    pub total: u32,
    /// The unit being measured, as the UI names it.
    pub testing: Option<String>,
}

/// The best unit found on one rung, or why there is none.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RungReport {
    pub rung: ComputeRung,
    pub usable: bool,
    pub device: Option<String>,
    /// The API and driver it runs through, e.g. "Vulkan 1.3 · Mesa 25.0.7".
    pub api: Option<String>,
    pub precision: Option<String>,
    /// Median time for one benchmark frame, hand-off to results.
    pub ms_per_frame: Option<f32>,
    /// Why it is not usable, when it is not.
    pub reason: Option<String>,
}

impl RungReport {
    pub fn absent(rung: ComputeRung, reason: impl Into<String>) -> Self {
        Self {
            rung,
            usable: false,
            device: None,
            api: None,
            precision: None,
            ms_per_frame: None,
            reason: Some(reason.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiComputeReport {
    /// Advances on every change, so a watcher can tell without comparing reports.
    pub generation: u64,
    pub state: BenchmarkState,
    pub progress: Option<BenchmarkProgress>,
    /// One entry per rung, in ladder order.
    pub rungs: Vec<RungReport>,
    /// What Auto picks: the fastest usable unit other than the CPU, else the CPU.
    pub auto: Option<ComputeRung>,
    /// Where the network runs for the preference asked about.
    pub effective: Option<ComputeRung>,
    /// Something the observer should know, e.g. a forced rung this machine cannot use.
    pub notice: Option<String>,
}

impl AiComputeReport {
    pub fn unavailable() -> Self {
        Self {
            generation: 0,
            state: BenchmarkState::Unavailable,
            progress: None,
            rungs: Vec::new(),
            auto: None,
            effective: None,
            notice: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_round_trip_as_snake_case() {
        for (preference, text) in [
            (AiComputePreference::Auto, "\"auto\""),
            (AiComputePreference::Npu, "\"npu\""),
            (AiComputePreference::DiscreteGpu, "\"discrete_gpu\""),
            (AiComputePreference::IntegratedGpu, "\"integrated_gpu\""),
            (AiComputePreference::Cpu, "\"cpu\""),
        ] {
            assert_eq!(serde_json::to_string(&preference).unwrap(), text);
            assert_eq!(serde_json::from_str::<AiComputePreference>(text).unwrap(), preference);
        }
    }

    /// A settings file from a newer build, a hand edit or a `null` must not fail the load.
    #[test]
    fn an_unknown_preference_reads_as_auto() {
        for text in ["\"tpu\"", "null", "3", "{}", "\"NPU\"", "\"\""] {
            assert_eq!(
                serde_json::from_str::<AiComputePreference>(text).unwrap(),
                AiComputePreference::Auto,
                "{text}"
            );
        }
    }

    #[test]
    fn the_ladder_runs_npu_first_and_cpu_last() {
        let mut sorted = ComputeRung::LADDER;
        sorted.sort();
        assert_eq!(sorted, ComputeRung::LADDER, "the derived order is the ladder's");
        assert_eq!(AiComputePreference::Auto.forced(), None);
        assert_eq!(AiComputePreference::Cpu.forced(), Some(ComputeRung::Cpu));
    }

    #[test]
    fn the_report_serialises_its_rungs_by_name() {
        let report = AiComputeReport {
            rungs: vec![RungReport::absent(ComputeRung::DiscreteGpu, "none found")],
            auto: Some(ComputeRung::Cpu),
            ..AiComputeReport::unavailable()
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["state"], "unavailable");
        assert_eq!(json["rungs"][0]["rung"], "discrete_gpu");
        assert_eq!(json["auto"], "cpu");
        assert_eq!(serde_json::from_value::<AiComputeReport>(json).unwrap(), report);
    }
}
