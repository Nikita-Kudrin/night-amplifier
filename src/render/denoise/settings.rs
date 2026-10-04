//! The observer's denoise settings: what `settings.json` and the API carry for the
//! denoise plugins, sanitised wherever the block enters or leaves.

use super::AiComputePreference;

/// Spatial denoising of the streamed image.
///
/// Runs in the encoders at the resolution the client asked for, not at sensor
/// resolution: the filters cost `1/4.5` as much there and the box downsample
/// has already halved the noise before they start. Both are off for
/// `StackingType::Planetary`, where the fine detail lucky imaging exists to
/// recover is exactly what a denoiser removes.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DenoiseSettings {
    /// The master switch, and a Pro control like the rest of this block. Off gives
    /// `DenoiseConfig::OFF`, which the encoders guarantee is *byte-identical* to the
    /// pre-denoise output rather than merely equivalent, so the toggle costs nothing.
    ///
    /// Deliberately **not** managed by Focus/Finder mode, unlike `chroma` and
    /// `luma_strength` (the mode already reaches off through those): forcing this one
    /// false once made a saved file migrate the Background Grain dial to `0.0` with
    /// nothing to put it back. The observer's master switch stays theirs.
    #[serde(default = "default_denoise_enabled")]
    pub enabled: bool,
    /// Guided-filter smoothing of the chroma planes, against luminance as the
    /// guide. Removes colour mottle; the eye resolves little chroma detail, so
    /// this is the cheap half with almost nothing to lose.
    #[serde(default = "default_chroma_denoise")]
    pub chroma: bool,
    /// How far the chroma planes move toward the filtered result, `0..=1`.
    #[serde(default = "default_denoise_strength", deserialize_with = "nullable_f32")]
    pub chroma_strength: f32,
    /// How hard the render fights sky grain, `0..=1`. The "Background Grain" control.
    ///
    /// Three mechanisms, spent cheapest-first by *scale* reached. On a 1440p IMX533
    /// stack the 16-32 and 32-64 px bands dominate perceived grain.
    ///
    /// - **Below the middle**: wavelet finest scale (`LumaDenoiseConfig::thresholds_for`)
    ///   + tone curve split (`AutoStretchConfig::grain_split`) between `MIN_GRAIN_SPLIT`
    ///   and `DEFAULT_GRAIN_SPLIT`. Trades brightness for fine speckle: moves 8-128 px
    ///   noise under 2%.
    /// - **Above the middle**: wavelet levels 5-6, the only mechanism reaching 16-64 px.
    ///   Six-session measurement: 8-128 px noise down 12-15% for 3% of target
    ///   brightness, vs. the curve's 1:1.
    ///
    /// Split stops at `DEFAULT_GRAIN_SPLIT`, never `MAX_GRAIN_SPLIT`: reaching 1/4 cost
    /// 38-43% of brightness for 37-39% of grain, the same 1:1 exchange — not a trade a
    /// user control should offer. Coarse levels replace that range. `0.5` is the middle;
    /// see [`DEFAULT_BACKGROUND_GRAIN`].
    #[serde(default = "default_background_grain", deserialize_with = "nullable_f32")]
    pub background_grain: f32,
    /// Scales the mid-scale wavelet thresholds (levels 2-4), `0..=1`. `1.0` is both the
    /// tuned value and the ceiling; `0.0` is the wavelet's genuine off switch.
    ///
    /// Note what this can and cannot reach: it leaves the finest level and the coarse
    /// pair alone, so it is the "the target still looks too noisy" control, not the "the
    /// sky still looks grainy" one — that is the Background Grain dial.
    #[serde(default = "default_denoise_strength", deserialize_with = "nullable_f32")]
    pub luma_strength: f32,
    /// How much the wavelet raises the target's own structure, `0..=1`. The "Detail"
    /// control; `0` renders exactly as before it existed.
    ///
    /// A gain on the levels [`luma_strength`](Self::luma_strength) thresholds, applied to
    /// what survived the threshold and held off the sky and every star by the plugin — so
    /// it rides on the brightness denoiser and does nothing while that is off.
    #[serde(default = "default_detail", deserialize_with = "nullable_f32")]
    pub detail: f32,
    /// The AI denoiser (Pro): a small network run after the stretch, taking over the
    /// finest four wavelet levels and Detail while it runs. Off by default. Needs the
    /// master switch on, like every filter here.
    ///
    /// Not managed by Focus/Finder mode either: the mode holds the network off where the
    /// frame's config is built (`stage_config::requested_denoise`), so this stays the
    /// observer's value — a switch the mode forced false once cost a saved file its dial.
    #[serde(default)]
    pub ai: bool,
    /// The "AI compute" choice (Pro): which unit runs the network. Auto takes the hardware
    /// benchmark's pick; a forced rung this machine cannot use falls back to Auto. Unknown
    /// values read as Auto ([`AiComputePreference`]'s own `Deserialize`).
    #[serde(default)]
    pub ai_compute: AiComputePreference,
}

/// Reads JSON `null` as NaN instead of failing. serde_json writes a non-finite `f32` as
/// `null` (and `1e39` over the API narrows to infinity), so one such value used to fail
/// the whole of `settings.json` on the next start — every camera, telescope and eyepiece
/// setting back to its default, made permanent by the next save. The block's
/// `sanitized()` turns the NaN into that field's default.
pub(crate) fn nullable_f32<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<f32, D::Error> {
    use serde::Deserialize;
    Ok(Option::<f32>::deserialize(deserializer)?.unwrap_or(f32::NAN))
}

pub(crate) fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}

/// The middle of the dial: the wavelet's finest scale fully spent, the tone curve at
/// `DEFAULT_GRAIN_SPLIT`, the coarse levels off.
///
/// **Not** the render of any earlier build, despite a first draft claiming so: the
/// pre-dial build defaulted `star_protection` to `1.0` (finest scale *untouched*) and
/// its split to `1/4`; the dial's middle is neither (`0.0` / `1/8`). On the 106-sub
/// IMX533 set, dial 0.0 renders the target core at 161 output levels vs. 141 at the
/// middle, and fine (1-4 px) sky noise at 0.51/0.50 vs. 0.24/0.31 — the middle is a tuned trade, not a reproduction of anything.
pub const DEFAULT_BACKGROUND_GRAIN: f32 = 0.5;

/// The Detail control's default, not an off switch: stars injected on every test target
/// stay within the plugin's guard here, where the top of the dial leaves a faint ring. See
/// the Pro repo's `plugins::denoise::detail`.
pub const DEFAULT_DETAIL: f32 = 0.5;

fn default_denoise_enabled() -> bool {
    true
}

fn default_chroma_denoise() -> bool {
    true
}

fn default_denoise_strength() -> f32 {
    1.0
}

fn default_background_grain() -> f32 {
    DEFAULT_BACKGROUND_GRAIN
}

fn default_detail() -> f32 {
    DEFAULT_DETAIL
}

impl Default for DenoiseSettings {
    fn default() -> Self {
        Self {
            enabled: default_denoise_enabled(),
            chroma: default_chroma_denoise(),
            chroma_strength: default_denoise_strength(),
            background_grain: default_background_grain(),
            luma_strength: default_denoise_strength(),
            detail: default_detail(),
            ai: false,
            ai_compute: AiComputePreference::Auto,
        }
    }
}

impl DenoiseSettings {
    /// These settings with the nonsense taken out: finite, and in `0..=1`.
    ///
    /// Applied wherever the block enters or leaves — `POST /api/settings`, loading and
    /// saving `settings.json` — so no reader ever sees a value `f32::clamp` would pass
    /// straight through (NaN), and none is ever written as `null` ([`nullable_f32`]).
    /// The plugin sanitises again for itself, since a test can hand it anything.
    pub fn sanitized(&self) -> Self {
        Self {
            enabled: self.enabled,
            chroma: self.chroma,
            chroma_strength: finite_or(self.chroma_strength, 1.0).clamp(0.0, 1.0),
            background_grain: finite_or(self.background_grain, DEFAULT_BACKGROUND_GRAIN)
                .clamp(0.0, 1.0),
            luma_strength: finite_or(self.luma_strength, 1.0).clamp(0.0, 1.0),
            detail: finite_or(self.detail, DEFAULT_DETAIL).clamp(0.0, 1.0),
            ai: self.ai,
            ai_compute: self.ai_compute,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A settings block Community never interprets still has to survive a round trip.
    ///
    /// The opposite of the lesson the `luma` switch taught: *those* fields were deleted
    /// from the format and deliberately not migrated. These are retained and ignored, so
    /// a Pro observer who runs Community once does not come back to a reset dial.
    #[test]
    fn community_keeps_the_pro_only_keys_it_does_not_read() {
        let tuned = DenoiseSettings {
            enabled: true,
            chroma: false,
            chroma_strength: 0.4,
            background_grain: 0.82,
            luma_strength: 0.3,
            detail: 0.9,
            ai: true,
            ai_compute: AiComputePreference::IntegratedGpu,
        };
        let round_tripped: DenoiseSettings =
            serde_json::from_str(&serde_json::to_string(&tuned).unwrap()).unwrap();
        assert_eq!(round_tripped, tuned);
    }

    /// NaN arrives over JSON and `f32::clamp` passes it straight through, where it
    /// reaches a tone-curve exponent and makes every comparison it meets false. Sanitised
    /// in Community so the plugin may assume finite values.
    #[test]
    fn sanitizing_takes_the_nonsense_out_wherever_it_came_from() {
        let wild = DenoiseSettings {
            enabled: true,
            chroma: true,
            chroma_strength: f32::NAN,
            background_grain: 7.0,
            luma_strength: -3.0,
            detail: f32::INFINITY,
            ai: true,
            ai_compute: AiComputePreference::Npu,
        };
        let clean = wild.sanitized();
        assert_eq!(clean.chroma_strength, 1.0, "NaN must fall back, not propagate");
        assert!(clean.ai, "sanitising must not drop the network switch");
        assert_eq!(clean.ai_compute, AiComputePreference::Npu, "nor the compute choice");
        assert_eq!(clean.background_grain, 1.0);
        assert_eq!(clean.luma_strength, 0.0);
        assert_eq!(clean.detail, DEFAULT_DETAIL, "an infinite dial is no position at all");
        assert!(clean.sanitized() == clean, "sanitising must be idempotent");

        let nan_dial = DenoiseSettings { background_grain: f32::NAN, ..Default::default() };
        assert_eq!(nan_dial.sanitized().background_grain, DEFAULT_BACKGROUND_GRAIN);
        let negative_detail = DenoiseSettings { detail: -0.5, ..Default::default() };
        assert_eq!(negative_detail.sanitized().detail, 0.0);
    }

    /// The dial is the only grain key there is, and a file missing it takes the middle.
    ///
    /// There is no migration behind this any more: the `luma` switch and the
    /// `star_protection` slider it replaced are gone from the format, so a file carrying
    /// them is a file from a build that no longer exists and its extra keys are ignored
    /// like any other unknown field. What still has to hold is that a partial or absent
    /// block lands on the defaults rather than on zero, since every one of these feeds a
    /// tone curve or a threshold table.
    #[test]
    fn a_settings_file_without_the_dial_takes_the_default() {
        let read = |json: serde_json::Value| -> DenoiseSettings {
            serde_json::from_value(json).unwrap()
        };

        assert_eq!(read(serde_json::json!({})), DenoiseSettings::default());
        assert_eq!(
            read(serde_json::json!({ "chroma": false })),
            DenoiseSettings { chroma: false, ..Default::default() },
            "one key present must not zero the rest"
        );

        let written = read(serde_json::json!({
            "chroma": true, "chroma_strength": 0.8,
            "background_grain": 0.75, "luma_strength": 0.5,
        }));
        assert_eq!(written.background_grain, 0.75);
        assert_eq!(written.luma_strength, 0.5);
        assert_eq!(
            written.detail, DEFAULT_DETAIL,
            "a file from before the Detail control takes its default, not zero"
        );
        assert!(!written.ai, "a file from before the network leaves it off");
        assert!(
            read(serde_json::json!({ "detail": null })).detail.is_nan(),
            "a null reads as NaN for `sanitized` to replace, never as a failed file"
        );
    }

    /// The saved shape is the keys the code reads and nothing else, and it reads
    /// back as itself.
    #[test]
    fn the_denoise_block_round_trips() {
        let settings = DenoiseSettings {
            enabled: true,
            chroma: false,
            chroma_strength: 0.25,
            background_grain: 0.8,
            luma_strength: 0.6,
            detail: 0.3,
            ai: true,
            ai_compute: AiComputePreference::Cpu,
        };
        let json = serde_json::to_value(&settings).unwrap();
        let mut keys: Vec<&str> = json.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "ai",
                "ai_compute",
                "background_grain",
                "chroma",
                "chroma_strength",
                "detail",
                "enabled",
                "luma_strength"
            ],
            "the saved block must carry the keys the code reads and nothing else"
        );
        assert_eq!(
            serde_json::from_value::<DenoiseSettings>(json).unwrap(),
            settings
        );
    }

    /// Files saved before the setting existed, and values this build does not know, load
    /// as Auto rather than failing the whole file.
    #[test]
    fn a_missing_or_unknown_ai_compute_reads_as_auto() {
        let read = |json: serde_json::Value| serde_json::from_value::<DenoiseSettings>(json).unwrap();
        assert_eq!(read(serde_json::json!({})).ai_compute, AiComputePreference::Auto);
        assert_eq!(read(serde_json::json!({ "ai_compute": "tpu" })).ai_compute, AiComputePreference::Auto);
        assert_eq!(read(serde_json::json!({ "ai_compute": null })).ai_compute, AiComputePreference::Auto);
        assert_eq!(
            read(serde_json::json!({ "ai_compute": "discrete_gpu" })).ai_compute,
            AiComputePreference::DiscreteGpu
        );
    }
}
