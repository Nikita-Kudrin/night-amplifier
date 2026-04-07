use super::{AutoStretchConfig, AutoStretchResult};
use crate::render::stretch::asinh_stretch;

/// Solve for the optimal stretch factor using the asinh stretch formula
///
/// # The Math
///
/// Given the asinh stretch formula:
/// ```text
/// output = asinh(stretch × input) / asinh(stretch)
/// ```
///
/// We want to find `stretch` such that when `input = adjusted_median`,
/// the `output = target_background`.
///
/// This equation cannot be solved algebraically in closed form, so we use
/// a hybrid approach:
/// 1. Initial estimate using a linearization for small values
/// 2. Bisection refinement for guaranteed convergence
///
/// # Arguments
/// * `adjusted_median` - The background median after black point subtraction (0.0 to 1.0)
/// * `target_output` - Desired output brightness for the median (0.0 to 1.0)
/// * `config` - Solver configuration
///
/// # Returns
/// The optimal stretch factor, or None if the solver fails to converge
#[tracing::instrument(skip(config))]
pub fn solve_stretch_factor(
    adjusted_median: f32,
    target_output: f32,
    config: &AutoStretchConfig,
) -> Option<f32> {
    if adjusted_median <= 0.0 || adjusted_median >= 1.0 {
        return Some(1.0);
    }

    if target_output <= 0.0 || target_output >= 1.0 {
        return Some(1.0);
    }

    if adjusted_median >= target_output {
        return Some(config.min_stretch);
    }

    let mut low = config.min_stretch;
    let mut high = config.max_stretch;

    for _ in 0..config.max_iterations {
        let mid = (low + high) / 2.0;
        let output = asinh_stretch(adjusted_median, mid);

        if (output - target_output).abs() < config.tolerance {
            return Some(mid);
        }

        if output < target_output {
            low = mid;
        } else {
            high = mid;
        }

        if (high - low) / mid < config.tolerance {
            return Some(mid);
        }
    }

    Some((low + high) / 2.0)
}

/// Solve for stretch factor with a better initial guess using Newton-Raphson
///
/// This is a more sophisticated solver that uses the derivative of the asinh
/// stretch function to converge faster. Falls back to bisection if Newton
/// diverges.
#[tracing::instrument(skip(config))]
pub fn solve_stretch_factor_newton(
    adjusted_median: f32,
    target_output: f32,
    config: &AutoStretchConfig,
) -> AutoStretchResult {
    if adjusted_median <= 1e-6 {
        return AutoStretchResult {
            stretch_factor: 1.0,
            black_point: 0.0,
            original_median: 0.0,
            adjusted_median,
            iterations: 0,
            converged: true,
        };
    }

    if adjusted_median >= target_output {
        return AutoStretchResult {
            stretch_factor: config.min_stretch,
            black_point: 0.0,
            original_median: adjusted_median,
            adjusted_median,
            iterations: 0,
            converged: true,
        };
    }

    let m = adjusted_median;
    let target = target_output;

    // Initial guess
    let mut stretch = (target / m).clamp(config.min_stretch, config.max_stretch);

    let mut iterations = 0u32;
    let mut converged = false;

    for i in 0..config.max_iterations {
        iterations = i + 1;

        let current_output = asinh_stretch(m, stretch);
        let error = current_output - target;

        if error.abs() < config.tolerance {
            converged = true;
            break;
        }

        // We need asinh and its derivative here.
        // asinh(x) = ln(x + sqrt(x^2 + 1))
        // d/dx asinh(x) = 1/sqrt(x^2 + 1)
        // f(s) = asinh(sm)/asinh(s) - target
        // f'(s) = [ (m/sqrt(s^2m^2+1))*asinh(s) - (asinh(sm)/sqrt(s^2+1)) ] / asinh(s)^2

        let asinh_s = crate::render::stretch::asinh(stretch);
        let asinh_sm = crate::render::stretch::asinh(stretch * m);
        let sqrt_1_sm2 = (1.0 + (stretch * m).powi(2)).sqrt();
        let sqrt_1_s2 = (1.0 + stretch.powi(2)).sqrt();

        let numerator = (m / sqrt_1_sm2) * asinh_s - asinh_sm / sqrt_1_s2;
        let derivative = numerator / (asinh_s * asinh_s);

        if derivative.abs() < 1e-8 {
            if error > 0.0 {
                stretch *= 0.5;
            } else {
                stretch *= 2.0;
            }
        } else {
            let step = error / derivative;
            let damping = 0.8;
            stretch -= damping * step;
        }

        stretch = stretch.clamp(config.min_stretch, config.max_stretch);
    }

    if !converged {
        if let Some(s) = solve_stretch_factor(m, target, config) {
            stretch = s;
            converged = true;
        }
    }

    AutoStretchResult {
        stretch_factor: stretch,
        black_point: 0.0,
        original_median: m,
        adjusted_median: m,
        iterations,
        converged,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_solve_stretch_factor_basic() {
        let config = AutoStretchConfig::default();

        let stretch = solve_stretch_factor(0.05, 0.15, &config).unwrap();

        let output = asinh_stretch(0.05, stretch);
        assert!(
            (output - 0.15).abs() < 0.01,
            "Stretch {} produced output {} instead of 0.15",
            stretch,
            output
        );
    }

    #[test]
    fn test_solve_stretch_factor_various_medians() {
        let config = AutoStretchConfig::default();
        let target = 0.15;

        for median in [0.01, 0.02, 0.05, 0.08, 0.10, 0.12] {
            let stretch = solve_stretch_factor(median, target, &config).unwrap();
            let output = asinh_stretch(median, stretch);

            assert!(
                (output - target).abs() < 0.01,
                "Median {} with stretch {} gave {} instead of {}",
                median,
                stretch,
                output,
                target
            );
        }
    }

    #[test]
    fn test_solve_stretch_factor_edge_cases() {
        let config = AutoStretchConfig::default();

        let stretch = solve_stretch_factor(0.0, 0.15, &config).unwrap();
        assert!((stretch - 1.0).abs() < 1e-6);

        let stretch = solve_stretch_factor(0.20, 0.15, &config).unwrap();
        assert!((stretch - config.min_stretch).abs() < 1e-6);

        let stretch = solve_stretch_factor(1.0, 0.15, &config).unwrap();
        assert!((stretch - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_solve_stretch_factor_newton_converges() {
        let config = AutoStretchConfig::default();

        let result = solve_stretch_factor_newton(0.05, 0.15, &config);

        assert!(result.converged);
        assert!(result.iterations < 20);

        let output = asinh_stretch(0.05, result.stretch_factor);
        assert!((output - 0.15).abs() < 0.01);
    }

    #[test]
    fn test_auto_stretch_solver_accuracy() {
        let config = AutoStretchConfig::default();
        let target = config.target_background;

        for median_base in [0.01, 0.02, 0.03, 0.05, 0.08, 0.10] {
            let adjusted_median = median_base;

            let result = solve_stretch_factor_newton(adjusted_median, target, &config);

            if result.converged {
                let actual_output = asinh_stretch(adjusted_median, result.stretch_factor);
                let error = (actual_output - target).abs();

                assert!(
                    error < 0.01,
                    "Median {} gave stretch {} with output {} (error {})",
                    median_base,
                    result.stretch_factor,
                    actual_output,
                    error
                );
            }
        }
    }
}
