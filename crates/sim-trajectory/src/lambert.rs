use crate::math::{dot, norm, scale, sub, Vector3};
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferDirection {
    ShortWay,
    LongWay,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LambertArc {
    pub departure_position_m: Vector3,
    pub arrival_position_m: Vector3,
    pub departure_velocity_mps: Vector3,
    pub arrival_velocity_mps: Vector3,
    pub time_of_flight_s: f64,
    pub central_mu_m3_s2: f64,
    pub iterations: u32,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum LambertError {
    #[error("NO_PHYSICAL_SOLUTION: {0}")]
    NoPhysicalSolution(String),
    #[error("NUMERICAL_NON_CONVERGENCE: {0}")]
    NumericalNonConvergence(String),
}

pub fn solve_lambert_universal(
    departure_position_m: Vector3,
    arrival_position_m: Vector3,
    time_of_flight_s: f64,
    central_mu_m3_s2: f64,
    direction: TransferDirection,
) -> Result<LambertArc, LambertError> {
    if !time_of_flight_s.is_finite() || time_of_flight_s <= 0.0 {
        return Err(LambertError::NoPhysicalSolution(
            "time of flight must be finite and positive".into(),
        ));
    }
    if !central_mu_m3_s2.is_finite() || central_mu_m3_s2 <= 0.0 {
        return Err(LambertError::NoPhysicalSolution(
            "central gravitational parameter must be finite and positive".into(),
        ));
    }
    if departure_position_m
        .iter()
        .chain(arrival_position_m.iter())
        .any(|value| !value.is_finite())
    {
        return Err(LambertError::NoPhysicalSolution(
            "endpoint positions must be finite".into(),
        ));
    }

    let departure_radius = norm(departure_position_m);
    let arrival_radius = norm(arrival_position_m);
    if departure_radius <= 0.0 || arrival_radius <= 0.0 {
        return Err(LambertError::NoPhysicalSolution(
            "Lambert endpoints cannot be at the central singularity".into(),
        ));
    }
    let cosine = (dot(departure_position_m, arrival_position_m)
        / (departure_radius * arrival_radius))
        .clamp(-1.0, 1.0);
    let sine_magnitude = (1.0 - cosine * cosine).max(0.0).sqrt();
    let sine = match direction {
        TransferDirection::ShortWay => sine_magnitude,
        TransferDirection::LongWay => -sine_magnitude,
    };
    let denominator = 1.0 - cosine;
    if denominator <= 1e-14 || sine.abs() <= 1e-14 {
        return Err(LambertError::NoPhysicalSolution(
            "collinear endpoints require a dedicated branch".into(),
        ));
    }
    let transfer_parameter = sine * (departure_radius * arrival_radius / denominator).sqrt();
    if transfer_parameter.abs() <= f64::EPSILON {
        return Err(LambertError::NoPhysicalSolution(
            "transfer geometry is singular".into(),
        ));
    }

    let target = central_mu_m3_s2.sqrt() * time_of_flight_s;
    let equation = |z: f64| -> Option<f64> {
        let c = stumpff_c(z);
        let s = stumpff_s(z);
        if !c.is_finite() || c <= 0.0 || !s.is_finite() {
            return None;
        }
        let y = departure_radius + arrival_radius + transfer_parameter * (z * s - 1.0) / c.sqrt();
        if !y.is_finite() || y < 0.0 {
            return None;
        }
        Some((y / c).powf(1.5) * s + transfer_parameter * y.sqrt() - target)
    };

    let lower_limit = -4.0 * PI * PI + 1e-8;
    let upper_limit = 64.0 * PI * PI;
    let samples = 4096_u32;
    let mut bracket = None;
    let mut previous: Option<(f64, f64)> = None;
    for index in 0..=samples {
        let fraction = f64::from(index) / f64::from(samples);
        let z = lower_limit + (upper_limit - lower_limit) * fraction;
        let Some(value) = equation(z) else {
            continue;
        };
        if value.abs() <= 1e-10 * target.max(1.0) {
            bracket = Some((z, z));
            break;
        }
        if let Some((previous_z, previous_value)) = previous {
            if previous_value.signum() != value.signum() {
                bracket = Some((previous_z, z));
                break;
            }
        }
        previous = Some((z, value));
    }
    let (mut lower, mut upper) = bracket.ok_or_else(|| {
        LambertError::NoPhysicalSolution("no zero-revolution universal-variable root".into())
    })?;

    let mut iterations = 0_u32;
    if lower != upper {
        let mut lower_value = equation(lower).ok_or_else(|| {
            LambertError::NumericalNonConvergence("lost lower root bracket".into())
        })?;
        for iteration in 1..=100_u32 {
            iterations = iteration;
            let middle = 0.5 * (lower + upper);
            let middle_value = equation(middle).ok_or_else(|| {
                LambertError::NumericalNonConvergence("invalid value inside root bracket".into())
            })?;
            if middle_value.abs() <= 1e-11 * target.max(1.0) || (upper - lower).abs() <= 1e-12 {
                lower = middle;
                upper = middle;
                break;
            }
            if lower_value.signum() == middle_value.signum() {
                lower = middle;
                lower_value = middle_value;
            } else {
                upper = middle;
            }
        }
    }
    let z = 0.5 * (lower + upper);
    let c = stumpff_c(z);
    let s = stumpff_s(z);
    let y = departure_radius + arrival_radius + transfer_parameter * (z * s - 1.0) / c.sqrt();
    if y <= 0.0 || !y.is_finite() {
        return Err(LambertError::NumericalNonConvergence(
            "root produced an invalid geometry parameter".into(),
        ));
    }
    let f = 1.0 - y / departure_radius;
    let g = transfer_parameter * (y / central_mu_m3_s2).sqrt();
    let g_dot = 1.0 - y / arrival_radius;
    if g.abs() <= 1e-12 || !g.is_finite() {
        return Err(LambertError::NumericalNonConvergence(
            "Lagrange g coefficient is singular".into(),
        ));
    }
    let departure_velocity_mps = scale(
        sub(arrival_position_m, scale(departure_position_m, f)),
        1.0 / g,
    );
    let arrival_velocity_mps = scale(
        sub(scale(arrival_position_m, g_dot), departure_position_m),
        1.0 / g,
    );
    if departure_velocity_mps
        .iter()
        .chain(arrival_velocity_mps.iter())
        .any(|value| !value.is_finite())
    {
        return Err(LambertError::NumericalNonConvergence(
            "solution velocity is non-finite".into(),
        ));
    }

    Ok(LambertArc {
        departure_position_m,
        arrival_position_m,
        departure_velocity_mps,
        arrival_velocity_mps,
        time_of_flight_s,
        central_mu_m3_s2,
        iterations,
    })
}

fn stumpff_c(z: f64) -> f64 {
    if z > 1e-8 {
        (1.0 - z.sqrt().cos()) / z
    } else if z < -1e-8 {
        ((-z).sqrt().cosh() - 1.0) / -z
    } else {
        0.5 - z / 24.0 + z * z / 720.0 - z * z * z / 40_320.0
    }
}

fn stumpff_s(z: f64) -> f64 {
    if z > 1e-8 {
        (z.sqrt() - z.sqrt().sin()) / z.powf(1.5)
    } else if z < -1e-8 {
        ((-z).sqrt().sinh() - (-z).sqrt()) / (-z).powf(1.5)
    } else {
        1.0 / 6.0 - z / 120.0 + z * z / 5040.0 - z * z * z / 362_880.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vallado_classic_case_matches_published_velocity() {
        let solution = solve_lambert_universal(
            [5_000_000.0, 10_000_000.0, 2_100_000.0],
            [-14_600_000.0, 2_500_000.0, 7_000_000.0],
            3_600.0,
            3.986_004_418e14,
            TransferDirection::ShortWay,
        )
        .unwrap();
        let expected_departure = [-5_992.495, 1_925.367, 3_245.638];
        let expected_arrival = [-3_312.459, -4_196.619, -385.289];
        for index in 0..3 {
            assert!(
                (solution.departure_velocity_mps[index] - expected_departure[index]).abs() < 0.02
            );
            assert!((solution.arrival_velocity_mps[index] - expected_arrival[index]).abs() < 0.02);
        }
    }
}
