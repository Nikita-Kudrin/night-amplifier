//! Push-To's state on the wire: the values its events carry, which the sync snapshot and
//! `GET /api/push-to/status` restate.

use serde::Serialize;

use night_amplifier_core::push_to::{DirectionHint, PushToDirectionResponse};

/// The target clients were told about.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "api-schema", derive(schemars::JsonSchema))]
pub struct PushToTarget {
    pub name: Option<String>,
    pub designation: Option<String>,
    pub ra_degrees: f64,
    pub dec_degrees: f64,
}

/// A push direction as announced: the plugin's answer plus the solved field of view.
/// `solved_camera` names the camera whose solve oriented `angle_deg` — see
/// [`PushToDirectionResponse::solved_camera`]. `stale`: the last solve failed, so these are
/// the numbers of the fix before it — stamped by `PushToAnnouncer`, which re-sends a held
/// direction marked when a solve fails, so a client's dismissed verdict cannot unmark it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "api-schema", derive(schemars::JsonSchema))]
pub struct PushDirection {
    pub angle_deg: f64,
    pub distance_deg: f64,
    pub direction_hint: DirectionHint,
    pub is_close: bool,
    pub fov_deg: Option<f64>,
    pub solved_camera: Option<String>,
    pub stale: bool,
}

impl PushDirection {
    pub fn new(direction: PushToDirectionResponse, fov_deg: Option<f64>) -> Self {
        Self {
            angle_deg: direction.angle_deg,
            distance_deg: direction.distance_deg,
            direction_hint: direction.direction_hint,
            is_close: direction.is_close,
            fov_deg,
            solved_camera: direction.solved_camera,
            stale: false,
        }
    }

    /// Rounded to the precision a person can act on: the direction is recomputed from
    /// floating-point spherical geometry every frame, so an exact comparison would let the
    /// per-frame spam straight back through. The solving camera is part of it: the same
    /// numbers from the other camera's solve point a different way on screen, and a fresh
    /// fix of the stale numbers is news.
    pub fn same_as(&self, other: &Self) -> bool {
        let key = |d: &Self| {
            (
                (d.angle_deg * 10.0).round() as i64,
                (d.distance_deg * 1000.0).round() as i64,
                d.is_close,
            )
        };
        key(self) == key(other) && self.solved_camera == other.solved_camera && self.stale == other.stale
    }
}

/// Which attempt of a multi-strategy solve is running — see
/// [`ServerEvent::PlateSolvingProgress`](super::ServerEvent::PlateSolvingProgress).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "api-schema", derive(schemars::JsonSchema))]
pub struct SolveStage {
    pub stage: String,
    pub attempt: usize,
    pub total: usize,
}

/// How the last solve ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "api-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SolveVerdict {
    Success,
    Failed,
    Cancelled,
}

/// Whether a plate solve is running, and how the last one ended.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "api-schema", derive(schemars::JsonSchema))]
pub struct SolveStatus {
    pub in_progress: bool,
    pub target_name: Option<String>,
    pub stage: Option<SolveStage>,
    /// `None` until a solve ends, and after a restart or a cleared target.
    pub last_result: Option<SolveVerdict>,
}

impl SolveStatus {
    pub(crate) fn finish(&mut self, verdict: Option<SolveVerdict>) {
        self.in_progress = false;
        self.stage = None;
        self.last_result = verdict;
    }
}
