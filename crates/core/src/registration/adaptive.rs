//! Adaptive registration strategies.
//!
//! Provides automatic configuration selection based on imaging conditions and
//! hint-based optimization for challenging scenarios.

use tracing::{field, instrument, Span};

use crate::detection::Star;
use crate::error::{Result, StackError};

use super::clutter::suppress_clutter;
use super::config::RegistrationConfig;
use super::engine::ImageRegistration;
use super::refine::refine_transform;
use super::support::Support;
use super::transform::AffineTransform;
use super::translation::vote_translation;

/// Result of adaptive registration including diagnostics.
///
/// Two fits: `transform` is the all-star refit the frame is warped with, while
/// `matched_stars` and `mean_residual` judge the rung's own fit, which is what exposes a
/// multi-image sub (see `register`). `support` judges the refit against chance.
#[derive(Debug, Clone)]
pub struct AdaptiveRegistrationResult {
    /// Target -> reference, refined over every star both lists hold.
    pub transform: AffineTransform,
    /// Stars the rung's fit pairs up.
    pub matched_stars: usize,
    /// Mean residual of those pairs under the rung's fit, in pixels.
    pub mean_residual: f32,
    /// How far `transform`'s pairs stand out from chance.
    pub support: Support,
    /// Configuration preset that succeeded.
    pub config_used: String,
    /// Number of attempts before success.
    pub attempts: usize,
}

impl AdaptiveRegistrationResult {
    /// Whether the fit is told apart from chance at a scale one session's subs can have
    /// (see [`Support::is_credible`]). A fit that is not is a coincidence, however many
    /// stars it claims.
    pub fn is_credible(&self) -> bool {
        self.support.is_credible(&self.transform)
    }
}

/// Adaptive registration that tries multiple strategies.
pub struct AdaptiveRegistration {
    configs: Vec<(String, RegistrationConfig)>,
}

impl Default for AdaptiveRegistration {
    fn default() -> Self {
        Self::new()
    }
}

/// Star cap for the live ladder's first rung. Triangle generation is O(n³) in star
/// count, so this sets the per-frame cost more than anything else in registration:
/// median time on the dense Orion fixture drops from 79.3ms (uncapped, 50 stars,
/// ~20,800 triangles) to 5.2ms at 30 stars (~4,000 triangles) to 2.1ms at 25 — but 25
/// fails twice as often as 30 or the uncapped preset. 30 is the smallest cap that
/// fails no more often than uncapped, sized for the Raspberry Pi 5 target.
const LIVE_FIRST_RUNG_MAX_STARS: usize = 30;

/// Name the translation-voting rung reports in `config_used`.
const TRANSLATION_RUNG: &str = "translation";

impl AdaptiveRegistration {
    /// Creates a new adaptive registration: two configs tried in order. `fast` used to
    /// lead but failed almost every frame (34/34 on the 250mm set, 15/19 on 130mm), so
    /// `default` runs first — equal or better fit quality, one attempt instead of two.
    ///
    /// First rung capped at [`LIVE_FIRST_RUNG_MAX_STARS`] rather than reordering by
    /// attempt count: discarded `fast` costs 0.01-0.07ms vs up to 80ms for uncapped
    /// `default` on a dense field. Fit quality is unchanged — RANSAC over
    /// correspondences decides accuracy, not star count.
    pub fn new() -> Self {
        Self {
            configs: vec![
                (
                    "default".to_string(),
                    RegistrationConfig::default().with_max_stars(LIVE_FIRST_RUNG_MAX_STARS),
                ),
                ("robust".to_string(), RegistrationConfig::robust()),
            ],
        }
    }

    /// Creates adaptive registration with full config set (slower but more thorough).
    pub fn thorough() -> Self {
        Self {
            configs: vec![
                ("default".to_string(), RegistrationConfig::default()),
                ("wide_field".to_string(), RegistrationConfig::wide_field()),
                (
                    "narrow_field".to_string(),
                    RegistrationConfig::narrow_field(),
                ),
                ("robust".to_string(), RegistrationConfig::robust()),
                ("permissive".to_string(), RegistrationConfig::permissive()),
            ],
        }
    }

    /// Adds a custom configuration to try.
    pub fn with_config(mut self, name: &str, config: RegistrationConfig) -> Self {
        self.configs.push((name.to_string(), config));
        self
    }

    /// Registers with no idea where the sub lies: [`Self::register_near`] without a prior.
    pub fn register(
        &self,
        ref_stars: &[Star],
        tgt_stars: &[Star],
    ) -> Result<AdaptiveRegistrationResult> {
        self.register_near(ref_stars, tgt_stars, None)
    }

    /// Registers using adaptive strategy, then refits the winner over every star in both
    /// lists.
    ///
    /// The ladder rungs see the lists with clutter suppressed (see `clutter`); the first
    /// credible fit wins. Failing that, voting gets a try, and must be credible too, for
    /// as many offsets as it searched (see `Support::searched`). Failing that, the first
    /// fit any rung produced is returned for the caller to refuse — it says the frame
    /// aligned to *something*, unlike an error.
    ///
    /// `prior` is the session's last accepted transform against the same reference:
    /// voting tries turns around its rotation, which is how it follows an alt-az field
    /// past its own ±0.6° reach (see `translation`).
    #[instrument(skip(self, ref_stars, tgt_stars, prior), fields(
        ref_count = ref_stars.len(),
        target_count = tgt_stars.len(),
        config_used = field::Empty,
        matched_stars = field::Empty,
        mean_residual = field::Empty,
        attempts = field::Empty,
        credible = field::Empty,
    ))]
    pub fn register_near(
        &self,
        ref_stars: &[Star],
        tgt_stars: &[Star],
        prior: Option<&AffineTransform>,
    ) -> Result<AdaptiveRegistrationResult> {
        let (ref_peaks, tgt_peaks) = (suppress_clutter(ref_stars), suppress_clutter(tgt_stars));
        let mut last_error = String::new();
        let mut fallback: Option<AdaptiveRegistrationResult> = None;

        for (attempt, (name, config)) in self.configs.iter().enumerate() {
            let registration = ImageRegistration::new(config.clone());
            match registration.register(&ref_peaks, &tgt_peaks) {
                Ok(ladder) => {
                    let result = self.finish(
                        (ref_stars, tgt_stars),
                        (&ref_peaks, &tgt_peaks),
                        &ladder,
                        config.max_residual,
                        name,
                        attempt + 1,
                    );
                    if result.is_credible() {
                        return Ok(record(result));
                    }
                    fallback.get_or_insert(result);
                }
                Err(e) => last_error = format!("{}: {}", name, e),
            }
        }

        let expected_rotation = prior.map_or(0.0, |prior| prior.rotation);
        if let Some(vote) = vote_translation(&ref_peaks, &tgt_peaks, expected_rotation) {
            let mut result = self.finish(
                (ref_stars, tgt_stars),
                (&ref_peaks, &tgt_peaks),
                &vote.transform,
                RegistrationConfig::default().max_residual,
                TRANSLATION_RUNG,
                self.configs.len() + 1,
            );
            result.support = result.support.searched(vote.trials);
            if result.is_credible() {
                return Ok(record(result));
            }
        }

        match fallback {
            Some(result) => Ok(record(result)),
            None => Err(StackError::Registration(format!(
                "All registration strategies failed. Last error: {}",
                last_error
            ))),
        }
    }

    /// A rung's fit made into a result: diagnosed as fitted, then refined and measured.
    ///
    /// The diagnostics judge the rung's own fit, not the refit: the refit locks onto one
    /// image of each star, so a multi-image sub that the rung fits loosely would read as
    /// clean and slip past `FrameGate` (one did, and lit the sky around every star in the
    /// stack). The rung fitted from at most `max_stars`; every star detected pins the
    /// refit down further.
    ///
    /// The refit and the support are each run on the full lists and on the
    /// clutter-suppressed ones, and the better-supported refit with its stronger support
    /// stands. Each list fails a different real sub: a nebula's noise maxima outnumber a
    /// thin sub's stars inside the refit's radius and drag it off (and inflate the
    /// expected coincidences: Cat's Eye true fits at −3 to −15 on full lists, −9 to −22
    /// suppressed), while M42's fat stars leave the suppressed lists few pairs. A
    /// coincidence is weak on both.
    fn finish(
        &self,
        (ref_stars, tgt_stars): (&[Star], &[Star]),
        (ref_peaks, tgt_peaks): (&[Star], &[Star]),
        fit: &AffineTransform,
        max_residual: f32,
        name: &str,
        attempts: usize,
    ) -> AdaptiveRegistrationResult {
        let (matched_stars, mean_residual) =
            self.compute_diagnostics(ref_stars, tgt_stars, fit, max_residual * 2.0);
        let supported = |transform: AffineTransform| {
            let support = [
                Support::measure(ref_stars, tgt_stars, &transform),
                Support::measure(ref_peaks, tgt_peaks, &transform),
            ]
            .into_iter()
            .min_by(|a, b| a.log10_chance.total_cmp(&b.log10_chance))
            .expect("two measurements");
            (transform, support)
        };
        let (transform, support) = [
            refine_transform(ref_stars, tgt_stars, fit),
            refine_transform(ref_peaks, tgt_peaks, fit),
        ]
        .into_iter()
        .map(supported)
        .min_by(|a, b| a.1.log10_chance.total_cmp(&b.1.log10_chance))
        .expect("two refits");
        AdaptiveRegistrationResult {
            transform,
            matched_stars,
            mean_residual,
            support,
            config_used: name.to_string(),
            attempts,
        }
    }

    /// Registers with hints about the expected image characteristics.
    pub fn register_with_hints(
        &self,
        ref_stars: &[Star],
        tgt_stars: &[Star],
        hints: &RegistrationHints,
    ) -> Result<AdaptiveRegistrationResult> {
        let mut prioritized_configs = Vec::new();

        let hint_config = hints.to_config();
        prioritized_configs.push(("hint_based".to_string(), hint_config));
        prioritized_configs.extend(self.configs.clone());

        let adaptive = AdaptiveRegistration {
            configs: prioritized_configs,
        };
        adaptive.register(ref_stars, tgt_stars)
    }

    fn compute_diagnostics(
        &self,
        ref_stars: &[Star],
        tgt_stars: &[Star],
        transform: &AffineTransform,
        threshold: f32,
    ) -> (usize, f32) {
        let correspondences =
            get_correspondences_for_transform(ref_stars, tgt_stars, transform, threshold);

        // Infinity, not zero: 0.0 is the *best* residual a fit can report, so a
        // caller folding this into a running median would read "no fit at all"
        // as "a perfect fit". `FrameGate` scores frames against exactly such a
        // median.
        if correspondences.is_empty() {
            return (0, f32::INFINITY);
        }

        let mean_residual = correspondences
            .iter()
            .map(|&(ri, ti)| transform.residual(&tgt_stars[ti], &ref_stars[ri]))
            .sum::<f32>()
            / correspondences.len() as f32;

        (correspondences.len(), mean_residual)
    }
}

/// Records `result` on the `register` span and hands it back.
fn record(result: AdaptiveRegistrationResult) -> AdaptiveRegistrationResult {
    let span = Span::current();
    span.record("config_used", result.config_used.as_str());
    span.record("matched_stars", result.matched_stars);
    // field::debug, not the bare f32: mean_residual is f32::INFINITY when
    // compute_diagnostics found no correspondences (see its comment). A non-finite
    // double attribute makes Jaeger's query API 500 on any trace search that touches it.
    span.record("mean_residual", field::debug(result.mean_residual));
    span.record("attempts", result.attempts);
    span.record("credible", result.is_credible());
    result
}

/// Hints about image characteristics to guide registration.
#[derive(Debug, Clone, Default)]
pub struct RegistrationHints {
    /// Expected field of view type.
    pub fov_type: FovType,
    /// Whether clouds or obstructions might be present.
    pub has_obstructions: bool,
    /// Whether significant field rotation is expected.
    pub has_rotation: bool,
    /// Whether satellite trails might be present.
    pub has_satellites: bool,
    /// Expected brightness variation.
    pub brightness_variation: BrightnessVariation,
}

impl RegistrationHints {
    /// Creates default hints.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets FOV type hint.
    pub fn with_fov(mut self, fov: FovType) -> Self {
        self.fov_type = fov;
        self
    }

    /// Sets obstruction hint.
    pub fn with_obstructions(mut self, has: bool) -> Self {
        self.has_obstructions = has;
        self
    }

    /// Sets rotation hint.
    pub fn with_rotation(mut self, has: bool) -> Self {
        self.has_rotation = has;
        self
    }

    /// Converts hints to a registration configuration.
    pub(crate) fn to_config(&self) -> RegistrationConfig {
        let mut config = match self.fov_type {
            FovType::Wide => RegistrationConfig::wide_field(),
            FovType::Standard => RegistrationConfig::default(),
            FovType::Narrow => RegistrationConfig::narrow_field(),
        };

        if self.has_obstructions || self.has_satellites {
            config.use_ransac = true;
            config.ransac_iterations = 200;
            config.ransac_threshold *= 1.5;
            config.descriptor_tolerance *= 1.5;
        }

        match self.brightness_variation {
            BrightnessVariation::Stable => {}
            BrightnessVariation::Moderate => {
                config.max_residual *= 1.5;
            }
            BrightnessVariation::High => {
                config.max_residual *= 2.0;
                config.min_matches = 3.max(config.min_matches - 1);
            }
        }

        config
    }
}

/// Field of view type hint.
#[derive(Debug, Clone, Copy, Default)]
pub enum FovType {
    /// Wide field (> 2 degrees).
    Wide,
    /// Standard field.
    #[default]
    Standard,
    /// Narrow field (< 0.5 degrees).
    Narrow,
}

/// Brightness variation hint.
#[derive(Debug, Clone, Copy, Default)]
pub enum BrightnessVariation {
    /// Stable brightness.
    #[default]
    Stable,
    /// Some variation (thin clouds, etc.).
    Moderate,
    /// High variation (clouds rolling, etc.).
    High,
}

/// Gets correspondences from a transform (for diagnostics).
fn get_correspondences_for_transform(
    ref_stars: &[Star],
    tgt_stars: &[Star],
    transform: &AffineTransform,
    threshold: f32,
) -> Vec<(usize, usize)> {
    let mut correspondences = Vec::new();
    let mut tgt_used = vec![false; tgt_stars.len()];

    for (ri, ref_star) in ref_stars.iter().enumerate() {
        let mut best_dist = f32::MAX;
        let mut best_ti = 0;

        for (ti, tgt_star) in tgt_stars.iter().enumerate() {
            if tgt_used[ti] {
                continue;
            }
            let dist = transform.residual(tgt_star, ref_star);
            if dist < best_dist {
                best_dist = dist;
                best_ti = ti;
            }
        }

        if best_dist < threshold {
            correspondences.push((ri, best_ti));
            tgt_used[best_ti] = true;
        }
    }

    correspondences
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_stars() -> Vec<Star> {
        vec![
            Star::new(100.0, 100.0, 1000.0, 0.9, 50.0),
            Star::new(200.0, 100.0, 900.0, 0.85, 45.0),
            Star::new(150.0, 200.0, 800.0, 0.8, 40.0),
            Star::new(250.0, 180.0, 700.0, 0.75, 35.0),
            Star::new(120.0, 280.0, 600.0, 0.7, 30.0),
        ]
    }

    #[test]
    fn test_adaptive_registration() {
        let ref_stars = create_test_stars();
        let tgt_stars: Vec<Star> = ref_stars
            .iter()
            .map(|s| Star::new(s.x + 5.0, s.y - 3.0, s.flux, s.peak, s.snr))
            .collect();

        let adaptive = AdaptiveRegistration::new();
        let result = adaptive.register(&ref_stars, &tgt_stars).unwrap();

        assert!(result.matched_stars >= 3);
        assert!(result.mean_residual < 5.0);
    }

    /// Triangle generation is O(n³) in the stars handed to the matcher, so the
    /// live ladder's first rung is what sets per-frame registration cost. An
    /// uncapped `default` (50 stars) measured 80 ms per frame on a dense field
    /// against `robust`'s 2 ms — pinned here because the cap is invisible at the
    /// call site and easy to lift back out while "tidying".
    #[test]
    fn the_live_ladder_caps_its_first_rung() {
        let adaptive = AdaptiveRegistration::new();
        let (name, first) = &adaptive.configs[0];

        assert_eq!(name, "default");
        assert!(
            first.max_stars <= LIVE_FIRST_RUNG_MAX_STARS,
            "first rung matches over {} stars; triangle generation is O(n³)",
            first.max_stars
        );
    }

    /// 0.0 is the *best* residual a fit can report. Handing it back for a
    /// transform nothing corresponds to lets a caller averaging residuals read
    /// "no fit" as "perfect fit" — `FrameGate` keeps exactly such a median.
    #[test]
    fn a_fit_with_no_correspondences_reports_an_infinite_residual() {
        let adaptive = AdaptiveRegistration::new();
        let ref_stars = create_test_stars();
        let tgt_stars = create_test_stars();

        // A transform that throws every star clean out of correspondence range.
        let nonsense = AffineTransform {
            tx: 1.0e6,
            ty: 1.0e6,
            ..AffineTransform::identity()
        };
        let (matched, residual) =
            adaptive.compute_diagnostics(&ref_stars, &tgt_stars, &nonsense, 3.0);

        assert_eq!(matched, 0);
        assert!(
            residual.is_infinite(),
            "no correspondences must not read as a perfect fit: {residual}"
        );
    }

    #[test]
    fn test_hints_config_generation() {
        let hints = RegistrationHints::new()
            .with_fov(FovType::Wide)
            .with_obstructions(true);

        let config = hints.to_config();

        assert!(config.use_ransac);
        assert_eq!(config.ransac_iterations, 200);
    }

    /// Deterministic pseudo-random stars inside `(x0, y0)..(x0 + size, y0 + size)`.
    fn scatter(count: usize, (x0, y0): (f32, f32), size: f32, flux: f32, seed: u64) -> Vec<Star> {
        let mut state = seed;
        let mut next = move || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (state >> 33) as f32 / (1u64 << 31) as f32
        };
        (0..count)
            .map(|_| Star::new(x0 + next() * size, y0 + next() * size, flux * (0.5 + next()), 0.5, 30.0))
            .collect()
    }

    /// A thin sub of a bright nebula, as the Cat's Eye fixture has them: the brightest
    /// detections are noise maxima on the nebula, different every sub, then 15 real
    /// stars, then noise. The ladder fitted these maxima to each other; the result has
    /// to be the real shift, and credible, with only 15 of 200 stars agreeing.
    #[test]
    fn a_thin_sub_of_a_bright_nebula_registers_on_its_few_stars() {
        let stars = scatter(15, (100.0, 100.0), 2800.0, 50.0, 1);
        let sub = |seed: u64, (dx, dy): (f32, f32)| -> Vec<Star> {
            let mut list = scatter(45, (1480.0 - dx, 1900.0 - dy), 50.0, 400.0, seed);
            list.extend(stars.iter().map(|s| Star::new(s.x - dx, s.y - dy, s.flux, s.peak, s.snr)));
            list.extend(scatter(140, (0.0, 0.0), 3000.0, 5.0, seed + 100));
            list.sort_by(|a, b| b.flux.total_cmp(&a.flux));
            list
        };
        let reference = sub(2, (0.0, 0.0));

        for (seed, shift) in [(3, (-4.0, 2.5)), (4, (-81.0, -37.0)), (5, (12.0, 160.0))] {
            let result = AdaptiveRegistration::new().register(&reference, &sub(seed, shift)).unwrap();
            let t = result.transform;
            assert!(result.is_credible(), "{shift:?}: {result:?}");
            assert!(
                (t.tx - shift.0).abs() < 0.5 && (t.ty - shift.1).abs() < 0.5 && t.rotation.abs() < 1e-3,
                "{shift:?}: registered at {t:?}"
            );
        }
    }

    /// When even the suppressed list's brightest are noise, the ladder has nothing to
    /// fit and the shift has to come from voting over every detection.
    #[test]
    fn stars_fainter_than_the_noise_register_by_voting() {
        let stars = scatter(15, (100.0, 100.0), 2800.0, 1.0, 8);
        let sub = |seed: u64, (dx, dy): (f32, f32)| -> Vec<Star> {
            let mut list = scatter(150, (0.0, 0.0), 3000.0, 100.0, seed);
            list.extend(stars.iter().map(|s| Star::new(s.x - dx, s.y - dy, s.flux, s.peak, s.snr)));
            list.sort_by(|a, b| b.flux.total_cmp(&a.flux));
            list
        };
        let reference = sub(9, (0.0, 0.0));

        let result = AdaptiveRegistration::new().register(&reference, &sub(10, (-30.0, 7.0))).unwrap();

        assert_eq!(result.config_used, TRANSLATION_RUNG);
        assert!(result.is_credible(), "{result:?}");
        assert!((result.transform.tx + 30.0).abs() < 0.5 && (result.transform.ty - 7.0).abs() < 0.5, "{result:?}");
    }

    /// The voting rung of an alt-az session, whose field turns ~0.1-0.7° a minute against
    /// the fixed reference: past ~0.3° a 3000 px frame's stars no longer agree on one
    /// shift (corners move 8+ px), and voting on shifts alone lost every Cat's Eye sub
    /// only it registers.
    #[test]
    fn a_thin_sub_registers_while_the_field_rotates() {
        let stars = scatter(15, (100.0, 100.0), 2800.0, 1.0, 8);
        let (theta, centre) = (0.5f32.to_radians(), 1500.0);
        let (sin, cos) = theta.sin_cos();
        let truth = AffineTransform::new(
            theta,
            1.0,
            centre - (cos * centre - sin * centre) - 30.0,
            centre - (sin * centre + cos * centre) + 7.0,
        );
        let sub = |seed: u64, transform: &AffineTransform| -> Vec<Star> {
            let mut list = scatter(150, (0.0, 0.0), 3000.0, 100.0, seed);
            list.extend(stars.iter().map(|s| {
                let (x, y) = transform.inverse_transform_point(s.x, s.y);
                Star::new(x, y, s.flux, s.peak, s.snr)
            }));
            list.sort_by(|a, b| b.flux.total_cmp(&a.flux));
            list
        };
        let reference = sub(9, &AffineTransform::identity());

        let result = AdaptiveRegistration::new().register(&reference, &sub(10, &truth)).unwrap();

        assert!(result.is_credible(), "{result:?}");
        for (x, y) in [(0.0, 0.0), (3000.0, 0.0), (0.0, 3000.0), (3000.0, 3000.0)] {
            let (found, expected) = (result.transform.transform_point(x, y), truth.transform_point(x, y));
            assert!(
                (found.0 - expected.0).hypot(found.1 - expected.1) < 1.0,
                "corner ({x}, {y}) lands at {found:?}, not {expected:?}: {result:?}"
            );
        }
    }

    /// One unrelated field in a few hundred used to pass as credible, all on the voting
    /// rung and all on 3-5 pairs: its peak is the best of ~65k shifts, and the Poisson
    /// tail prices each pair as if the shift had been fixed beforehand. A swung Dob
    /// whose new field happens to be one of those stacks it into the old stack, and
    /// every later sub of that field repeats the coincidence. (The fixtures' real voting
    /// fits score -7.0 to -22 once they have paid for their trials.)
    #[test]
    fn unrelated_sparse_fields_are_not_credible_by_voting() {
        let registration = AdaptiveRegistration::new();
        let credible: Vec<_> = (0..500u64)
            .filter_map(|seed| {
                let reference = scatter(200, (0.0, 0.0), 3008.0, 10.0, 2 * seed + 11_000);
                let target = scatter(50, (0.0, 0.0), 3008.0, 10.0, 2 * seed + 11_001);
                registration.register(&reference, &target).ok().filter(|r| r.is_credible())
            })
            .map(|r| (r.config_used, r.support.pairs, r.support.log10_chance))
            .collect();

        assert!(credible.is_empty(), "{} of 500 unrelated pairs credible: {credible:?}", credible.len());
    }

    /// Two different fields must not register credibly, whichever rung tries.
    #[test]
    fn unrelated_fields_get_no_credible_fit() {
        let reference = scatter(200, (0.0, 0.0), 3000.0, 10.0, 6);
        let other = scatter(200, (0.0, 0.0), 3000.0, 10.0, 7);

        let credible = AdaptiveRegistration::new()
            .register(&reference, &other)
            .is_ok_and(|result| result.is_credible());

        assert!(!credible);
    }
}
