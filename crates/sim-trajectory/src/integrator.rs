use crate::math::{add, norm, scale, sub, Vector3};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CartesianState {
    pub position_m: Vector3,
    pub velocity_mps: Vector3,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct IntegratorOptions {
    pub position_absolute_tolerance_m: f64,
    pub velocity_absolute_tolerance_mps: f64,
    pub relative_tolerance: f64,
    pub maximum_steps: u32,
}

impl Default for IntegratorOptions {
    fn default() -> Self {
        Self {
            position_absolute_tolerance_m: 10.0,
            velocity_absolute_tolerance_mps: 0.01,
            relative_tolerance: 1e-9,
            maximum_steps: 100_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntegrationResult {
    pub final_state: CartesianState,
    pub accepted_steps: u32,
    pub rejected_steps: u32,
    pub maximum_normalized_local_error: f64,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum IntegrationError {
    #[error("NO_PHYSICAL_SOLUTION: {0}")]
    InvalidInput(String),
    #[error("NUMERICAL_NON_CONVERGENCE: adaptive integrator exceeded {0} steps")]
    StepBudgetExceeded(u32),
    #[error("NUMERICAL_NON_CONVERGENCE: adaptive step fell below minimum")]
    MinimumStep,
    #[error("NUMERICAL_NON_CONVERGENCE: integration produced a non-finite state")]
    NonFinite,
}

pub fn integrate_two_body_adaptive(
    initial_state: CartesianState,
    duration_s: f64,
    central_mu_m3_s2: f64,
    options: IntegratorOptions,
) -> Result<IntegrationResult, IntegrationError> {
    if !duration_s.is_finite()
        || duration_s <= 0.0
        || !central_mu_m3_s2.is_finite()
        || central_mu_m3_s2 <= 0.0
    {
        return Err(IntegrationError::InvalidInput(
            "duration and central gravity must be finite and positive".into(),
        ));
    }
    if initial_state
        .position_m
        .iter()
        .chain(initial_state.velocity_mps.iter())
        .any(|value| !value.is_finite())
        || norm(initial_state.position_m) <= 0.0
    {
        return Err(IntegrationError::InvalidInput(
            "initial state must be finite and outside the central singularity".into(),
        ));
    }
    if options.position_absolute_tolerance_m <= 0.0
        || options.velocity_absolute_tolerance_mps <= 0.0
        || options.relative_tolerance <= 0.0
        || options.maximum_steps == 0
    {
        return Err(IntegrationError::InvalidInput(
            "integrator tolerances and step budget must be positive".into(),
        ));
    }

    let minimum_step = (duration_s * 1e-12).max(1e-6);
    let maximum_step = (duration_s / 8.0).max(minimum_step);
    let mut step = (duration_s / 128.0).clamp(minimum_step, maximum_step);
    let mut elapsed = 0.0;
    let mut state = initial_state;
    let mut accepted = 0_u32;
    let mut rejected = 0_u32;
    let mut maximum_error = 0.0_f64;

    while elapsed < duration_s {
        if accepted + rejected >= options.maximum_steps {
            return Err(IntegrationError::StepBudgetExceeded(options.maximum_steps));
        }
        step = step.min(duration_s - elapsed);
        if step < minimum_step && duration_s - elapsed > minimum_step {
            return Err(IntegrationError::MinimumStep);
        }
        let full = rk4_step(state, step, central_mu_m3_s2)?;
        let first_half = rk4_step(state, step * 0.5, central_mu_m3_s2)?;
        let second_half = rk4_step(first_half, step * 0.5, central_mu_m3_s2)?;
        let normalized_error = normalized_error(state, full, second_half, options);
        maximum_error = maximum_error.max(normalized_error);

        if normalized_error <= 1.0 {
            state = CartesianState {
                position_m: add(
                    second_half.position_m,
                    scale(sub(second_half.position_m, full.position_m), 1.0 / 15.0),
                ),
                velocity_mps: add(
                    second_half.velocity_mps,
                    scale(sub(second_half.velocity_mps, full.velocity_mps), 1.0 / 15.0),
                ),
            };
            elapsed += step;
            accepted += 1;
        } else {
            rejected += 1;
        }
        let factor = if normalized_error <= f64::EPSILON {
            2.0
        } else {
            (0.9 * normalized_error.powf(-0.2)).clamp(0.2, 2.0)
        };
        step = (step * factor).clamp(minimum_step, maximum_step);
    }

    Ok(IntegrationResult {
        final_state: state,
        accepted_steps: accepted,
        rejected_steps: rejected,
        maximum_normalized_local_error: maximum_error,
    })
}

fn rk4_step(
    state: CartesianState,
    step_s: f64,
    central_mu_m3_s2: f64,
) -> Result<CartesianState, IntegrationError> {
    let derivative = |value: CartesianState| -> Result<CartesianState, IntegrationError> {
        let radius = norm(value.position_m);
        if !radius.is_finite() || radius <= 0.0 {
            return Err(IntegrationError::NonFinite);
        }
        Ok(CartesianState {
            position_m: value.velocity_mps,
            velocity_mps: scale(value.position_m, -central_mu_m3_s2 / radius.powi(3)),
        })
    };
    let offset = |base: CartesianState, slope: CartesianState, factor: f64| CartesianState {
        position_m: add(base.position_m, scale(slope.position_m, factor)),
        velocity_mps: add(base.velocity_mps, scale(slope.velocity_mps, factor)),
    };
    let k1 = derivative(state)?;
    let k2 = derivative(offset(state, k1, step_s * 0.5))?;
    let k3 = derivative(offset(state, k2, step_s * 0.5))?;
    let k4 = derivative(offset(state, k3, step_s))?;
    let combine = |a: Vector3, b: Vector3, c: Vector3, d: Vector3| {
        add(a, scale(add(add(b, c), d), step_s / 6.0))
    };
    let next = CartesianState {
        position_m: combine(
            state.position_m,
            k1.position_m,
            scale(k2.position_m, 2.0),
            add(scale(k3.position_m, 2.0), k4.position_m),
        ),
        velocity_mps: combine(
            state.velocity_mps,
            k1.velocity_mps,
            scale(k2.velocity_mps, 2.0),
            add(scale(k3.velocity_mps, 2.0), k4.velocity_mps),
        ),
    };
    if next
        .position_m
        .iter()
        .chain(next.velocity_mps.iter())
        .any(|value| !value.is_finite())
    {
        Err(IntegrationError::NonFinite)
    } else {
        Ok(next)
    }
}

fn normalized_error(
    initial: CartesianState,
    coarse: CartesianState,
    fine: CartesianState,
    options: IntegratorOptions,
) -> f64 {
    let position_error = norm([
        fine.position_m[0] - coarse.position_m[0],
        fine.position_m[1] - coarse.position_m[1],
        fine.position_m[2] - coarse.position_m[2],
    ]) / 15.0;
    let velocity_error = norm([
        fine.velocity_mps[0] - coarse.velocity_mps[0],
        fine.velocity_mps[1] - coarse.velocity_mps[1],
        fine.velocity_mps[2] - coarse.velocity_mps[2],
    ]) / 15.0;
    let position_scale = options.position_absolute_tolerance_m
        + options.relative_tolerance * norm(initial.position_m).max(norm(fine.position_m));
    let velocity_scale = options.velocity_absolute_tolerance_mps
        + options.relative_tolerance * norm(initial.velocity_mps).max(norm(fine.velocity_mps));
    (position_error / position_scale).max(velocity_error / velocity_scale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::TAU;

    #[test]
    fn circular_orbit_returns_to_start() {
        let mu = 3.986_004_418e14;
        let radius: f64 = 7_000_000.0;
        let speed = (mu / radius).sqrt();
        let period = TAU * (radius.powi(3) / mu).sqrt();
        let result = integrate_two_body_adaptive(
            CartesianState {
                position_m: [radius, 0.0, 0.0],
                velocity_mps: [0.0, speed, 0.0],
            },
            period,
            mu,
            IntegratorOptions {
                position_absolute_tolerance_m: 0.01,
                velocity_absolute_tolerance_mps: 1e-5,
                relative_tolerance: 1e-12,
                maximum_steps: 100_000,
            },
        )
        .unwrap();
        let position_error = norm([
            result.final_state.position_m[0] - radius,
            result.final_state.position_m[1],
            result.final_state.position_m[2],
        ]);
        let velocity_error = norm([
            result.final_state.velocity_mps[0],
            result.final_state.velocity_mps[1] - speed,
            result.final_state.velocity_mps[2],
        ]);
        assert!(position_error < 20.0, "position error {position_error}");
        assert!(velocity_error < 0.02, "velocity error {velocity_error}");
    }
}
