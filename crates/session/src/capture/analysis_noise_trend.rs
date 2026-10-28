//! How a stack's noise figure moves between measurements: along its own fall with depth,
//! and continuously through the windows `auto_crop` cuts it to.

/// Steepest fall of the stack's noise with depth a trend may extrapolate.
///
/// `1/sqrt(N)` is pure averaging of independent subs, and nothing a stack does falls
/// faster; real stacks fall slower (the 106-sub IMX533 set: `N^-0.41` to 32 subs, then
/// `N^-0.19`) because sky structure under the MAD does not average away.
const MAX_NOISE_EXPONENT: f32 = 0.5;

/// The most one crop move may change the noise figure through the same estimates: past it
/// the probe misread, and the move is not a window's. Measured moves stayed within 14 % on
/// 18 sessions.
const MAX_WINDOW_STEP: f32 = 1.5;

/// The most all of a stack's moves may scale its noise figure, either way: a backstop, as
/// each is measured two-sided. The 106-sub dumbbell's five moves reached 0.83.
const MAX_WINDOW: f32 = 2.0;

/// How the stack's noise falls with depth, from its last two measurements.
///
/// Between refreshes a reused MAD is carried from the depth it was measured at to the
/// frame's, as `sigma ∝ N^-exponent`: the curve then moves every frame as the stack does,
/// and a refresh corrects only the trend's error. The exponent is clamped to
/// `[0, MAX_NOISE_EXPONENT]` — a measurement that rose (a cloud, a bright sub) holds
/// rather than predicting noise that grows with depth. Over the most a trend is carried,
/// a quarter's growth, a clamped extreme moves the MAD at most 12 %.
#[derive(Debug, Clone, Copy)]
pub(super) struct NoiseTrend {
    /// Depth and mean sigma, as served, of the latest measurement.
    latest: Option<(u32, f32)>,
    exponent: f32,
    /// What every measurement is served at: how far each window the stack's crop moved to
    /// read from the trend, through the estimates the last one was measured with.
    window: f32,
}

impl Default for NoiseTrend {
    fn default() -> Self {
        Self { latest: None, exponent: 0.0, window: 1.0 }
    }
}

impl NoiseTrend {
    /// A trend that serves every measurement at `window`: a one-shot render of the stack
    /// the render task carried that far ([`Self::window`]).
    pub(super) fn through(window: f32) -> Self {
        Self { window: if window.is_finite() && window > 0.0 { window } else { 1.0 }, ..Self::default() }
    }

    /// Records a measurement of `sigma` at `depth` and returns the factor it is served at.
    pub(super) fn observe(&mut self, depth: u32, sigma: f32) -> f32 {
        if depth == 0 || !sigma.is_finite() || sigma <= 0.0 {
            return self.window;
        }
        let served = sigma * self.window;
        if let Some((previous_depth, previous_sigma)) = self.latest {
            if depth > previous_depth {
                let exponent = (previous_sigma / served).ln() / (depth as f32 / previous_depth as f32).ln();
                self.exponent = exponent.clamp(0.0, MAX_NOISE_EXPONENT);
            }
        }
        self.latest = Some((depth, served));
        self.window
    }

    /// The crop moved, and the stack read `left` on the crop it left and `entered` on the one
    /// it moved to, through the same estimates: what the window alone changed. The figure is
    /// kept where it was, and the ratio with it: the frame MAD reads the target's structure as
    /// well as noise, so a crop cutting sky rose +14 % in one sub (the 106-sub dumbbell at 41,
    /// 3.55 -> 4.05 ADU) and gave back 3 levels of the target's contrast. Then a rebase: the
    /// refit that frame is served is the stack's own change, not the window's.
    pub(super) fn move_window(&mut self, left: f32, entered: f32) {
        let step = left / entered;
        if (1.0 / MAX_WINDOW_STEP..=MAX_WINDOW_STEP).contains(&step) {
            self.window = (self.window * step).clamp(1.0 / MAX_WINDOW, MAX_WINDOW);
        }
        self.rebase();
    }

    /// Forgets the latest measurement but keeps the exponent and the window: the next one
    /// sees the frame changed (a refit model, a plate solve's footprints), and an exponent
    /// derived across the two would read that change as the stack deepening.
    pub(super) fn rebase(&mut self) {
        self.latest = None;
    }

    /// A crop that starts or ends: the border it cuts or restores is thin coverage, whose
    /// noise inflated the MAD 13-20 % (and an exponent derived across it read the removal as
    /// the stack deepening). Its step is noise, so it is served, from a window of 1.
    pub(super) fn unwindow(&mut self) {
        self.rebase();
        self.window = 1.0;
    }

    /// Factor carrying the latest measurement's noise to `depth`.
    pub(super) fn scale_to(&self, depth: u32) -> f32 {
        match self.latest {
            Some((measured_at, _)) if depth > measured_at => (measured_at as f32 / depth as f32).powf(self.exponent),
            _ => 1.0,
        }
    }

    /// What measurements are served at.
    pub(super) fn window(&self) -> f32 {
        self.window
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trend measured at 16 and 25 subs falling as `N^-0.3` from 0.004.
    fn falling() -> NoiseTrend {
        let mut t = NoiseTrend::default();
        t.observe(16, 0.004);
        t.observe(25, 0.004 * (16.0f32 / 25.0).powf(0.3));
        t
    }

    /// Without a moved window every measurement is served as taken, bit for bit.
    #[test]
    fn an_unmoved_window_serves_measurements_as_taken() {
        let mut t = falling();
        assert_eq!(t.observe(31, 0.0031), 1.0);
        t.rebase();
        assert_eq!(t.observe(40, 0.0029), 1.0);
    }

    /// The deep dumbbell at 41: the window moves and the stack reads 14 % more noise on the
    /// crop it entered than on the one it left. The figure keeps the ratio, and the
    /// measurements after keep it too, a refit's own step included but not bridged.
    #[test]
    fn a_moved_window_keeps_the_noise_figure_where_it_was() {
        let mut t = falling();
        t.move_window(0.0036, 0.0036 * 1.14);
        assert!((t.window() - 1.0 / 1.14).abs() < 1e-5, "window {}", t.window());

        let refit = 0.0036 * 1.14 * 0.9;
        assert_eq!(t.observe(30, refit), t.window(), "the refit is served through the window");
        assert!((t.exponent - 0.3).abs() < 1e-4, "the refit's step bent the trend: {}", t.exponent);
        assert!((t.scale_to(36) - (30.0f32 / 36.0).powf(0.3)).abs() < 1e-6);

        let window = t.window();
        t.move_window(0.0031, 0.0031 * 0.94);
        assert!((t.window() - window / 0.94).abs() < 1e-5, "windows compound: {} after {window}", t.window());
    }

    /// A stack's first move needs nothing measured before it: both sides are read now.
    #[test]
    fn a_first_move_needs_no_measurement_before_it() {
        let mut t = NoiseTrend::default();
        t.move_window(0.004, 0.005);
        assert_eq!(t.observe(20, 0.005), 0.004 / 0.005);
    }

    /// A move the probe misread by more than any window does, or could not read, keeps the
    /// window it had; and no run of moves takes the window past `MAX_WINDOW` either way.
    #[test]
    fn a_misread_move_keeps_the_window_and_moves_stay_bounded() {
        let mut t = falling();
        for (left, entered) in [(0.004, 0.008), (0.004, 0.0), (f32::NAN, 0.004), (0.004, f32::INFINITY)] {
            t.move_window(left, entered);
            assert_eq!(t.window(), 1.0, "{left} / {entered}");
        }
        assert_eq!(t.scale_to(31), 1.0, "a misread move still rebases");
        for _ in 0..10 {
            t.move_window(0.003, 0.004);
        }
        assert_eq!(t.window(), 1.0 / MAX_WINDOW);
        for _ in 0..20 {
            t.move_window(0.004, 0.003);
        }
        assert_eq!(t.window(), MAX_WINDOW);
    }

    /// A crop starting or ending, or a trend reset, serves what it measures again.
    #[test]
    fn a_crop_starting_or_ending_serves_the_frame_as_measured() {
        let mut t = falling();
        t.move_window(0.004, 0.0042);
        assert!(t.observe(30, 0.004) < 1.0);
        t.unwindow();
        assert_eq!(t.observe(31, 0.004), 1.0);
        assert_eq!(NoiseTrend::through(0.8).observe(31, 0.004), 0.8);
        assert_eq!(NoiseTrend::through(f32::NAN).window(), 1.0);
    }
}
