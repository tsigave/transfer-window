//! Reproducible transfer search, finite-thrust engineering checks, and independent verification.

mod integrator;
mod lambert;
mod math;
mod pareto;

pub use integrator::{
    integrate_two_body_adaptive, CartesianState, IntegrationError, IntegrationResult,
    IntegratorOptions,
};
pub use lambert::{solve_lambert_universal, LambertArc, LambertError, TransferDirection};
pub use pareto::{pareto_front, select_representatives, ParetoObjectives, RepresentativeSolutions};

use crate::math::{norm, sub, unit, Vector3};
use serde::{Deserialize, Serialize};
use sim_astro::{AstroError, Catalog, DevelopmentStatus, EphemerisService, StateVector};
use sim_engineering::{
    apply_burn, BurnOutcome, BurnRequest, ConstraintViolation, DurationSeconds, EnergyJoules,
    EngineeringCatalog, MassKilograms, PowerWatts, ReservePolicy, ShipBlueprint,
    VelocityMetersPerSecond, VesselState, VolumeCubicMeters,
};
use sim_time::{StableId, TdbInstant, MICROS_PER_SECOND};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub const SOLVER_VERSION: &str = "transfer-window-trajectory-v1";
pub const GRAVITATIONAL_CONSTANT_M3_KG_S2: f64 = 6.674_30e-11;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArrivalCondition {
    Rendezvous,
    Flyby,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TimeWindow {
    pub earliest: TdbInstant,
    pub latest: TdbInstant,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DurationWindow {
    pub minimum_s: f64,
    pub maximum_s: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolverOptions {
    pub departure_samples: u32,
    pub duration_samples: u32,
    pub maximum_evaluations: u32,
    pub direction: TransferDirection,
    pub verification_position_tolerance_m: f64,
    pub verification_velocity_tolerance_mps: f64,
    pub integrator: IntegratorOptions,
}

impl Default for SolverOptions {
    fn default() -> Self {
        Self {
            departure_samples: 3,
            duration_samples: 5,
            maximum_evaluations: 64,
            direction: TransferDirection::ShortWay,
            verification_position_tolerance_m: 2_000_000.0,
            verification_velocity_tolerance_mps: 2.0,
            integrator: IntegratorOptions::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransferRequest {
    pub origin_id: StableId,
    pub destination_id: StableId,
    pub departure_window: TimeWindow,
    pub duration_window: DurationWindow,
    pub vessel_id: StableId,
    pub payload_mass_kg: MassKilograms,
    pub payload_volume_m3: VolumeCubicMeters,
    pub reserve_policy: ReservePolicy,
    pub arrival_condition: ArrivalCondition,
    pub options: SolverOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchedConicEventKind {
    DepartureEscape,
    HeliocentricTransfer,
    ArrivalApproach,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatchedConicEvent {
    pub kind: PatchedConicEventKind,
    pub body_id: Option<StableId>,
    pub epoch: TdbInstant,
    pub hyperbolic_excess_velocity_mps: f64,
    pub local_maneuver_delta_v_mps: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TrajectorySegment {
    FiniteBurn {
        phase: String,
        start: TdbInstant,
        end: TdbInstant,
        direction: Vector3,
        target_delta_v_mps: f64,
        thrust_n: f64,
        effective_exhaust_velocity_mps: f64,
        input_power_w: f64,
        powered_duration_s: f64,
        elapsed_duration_s: f64,
        chunk_count: u32,
        initial_mass_kg: f64,
        final_mass_kg: f64,
        peak_waste_heat_w: f64,
    },
    Coast {
        start: TdbInstant,
        end: TdbInstant,
        start_state: StateVector,
        end_state: StateVector,
    },
    Approach {
        at: TdbInstant,
        destination_id: StableId,
        planned_position_error_m: f64,
        planned_velocity_error_mps: f64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationLevel {
    Candidate,
    Executable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TerminationReason {
    Converged,
    NoPhysicalSolution,
    NumericalNonConvergence,
    ConstraintViolation,
    SearchBudgetExhausted,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolverMetadata {
    pub input_hash: String,
    pub solver_version: String,
    pub lambert_iterations: u32,
    pub integrator_accepted_steps: u32,
    pub integrator_rejected_steps: u32,
    pub position_tolerance_m: f64,
    pub velocity_tolerance_mps: f64,
    pub termination_reason: TerminationReason,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeasibilityMargins {
    pub position_error_m: f64,
    pub velocity_error_mps: f64,
    pub propellant_remaining_kg: f64,
    pub fusion_fuel_remaining_kg: f64,
    pub reactor_lifetime_remaining_s: f64,
    pub engine_lifetime_remaining_s: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationServices {
    pub market: bool,
    pub propellant_supply: bool,
    pub repair: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransferSolution {
    pub id: StableId,
    pub origin_id: StableId,
    pub destination_id: StableId,
    pub departure: TdbInstant,
    pub arrival: TdbInstant,
    pub time_of_flight_s: f64,
    pub payload_mass_kg: f64,
    pub lambert_arc: LambertArc,
    pub patched_conic_events: Vec<PatchedConicEvent>,
    pub segments: Vec<TrajectorySegment>,
    pub propellant_consumed_kg: f64,
    pub fusion_fuel_consumed_kg: f64,
    pub peak_power_w: f64,
    pub peak_waste_heat_w: f64,
    pub reactor_lifetime_used_s: f64,
    pub engine_lifetime_used_s: f64,
    pub estimated_cost_credits: f64,
    pub margins: FeasibilityMargins,
    pub destination_services: DestinationServices,
    pub validation_level: ValidationLevel,
    pub metadata: SolverMetadata,
}

impl TransferSolution {
    pub fn pareto_objectives(&self) -> ParetoObjectives {
        ParetoObjectives {
            arrival_tdb_micros: self.arrival.micros_since_j2000(),
            propellant_kg: self.propellant_consumed_kg,
            payload_kg: self.payload_mass_kg,
            lifetime_used_s: self.reactor_lifetime_used_s + self.engine_lifetime_used_s,
            estimated_cost_credits: self.estimated_cost_credits,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateFailureKind {
    InvalidRequest,
    NoPhysicalSolution,
    NumericalNonConvergence,
    ConstraintViolation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateFailure {
    pub departure: Option<TdbInstant>,
    pub duration_s: Option<f64>,
    pub kind: CandidateFailureKind,
    pub message: String,
    pub constraints: Vec<ConstraintViolation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchStatus {
    Completed,
    PartialBudgetExhausted,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchProgress {
    pub evaluated: u32,
    pub planned: u32,
    pub executable_solutions: usize,
    pub status: SearchStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransferSearchReport {
    pub input_hash: String,
    pub solutions: Vec<TransferSolution>,
    pub failures: Vec<CandidateFailure>,
    pub evaluated: u32,
    pub planned: u32,
    pub status: SearchStatus,
    pub termination_reason: TerminationReason,
}

#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TrajectoryError {
    #[error("INVALID_REQUEST: {0}")]
    InvalidRequest(String),
    #[error(transparent)]
    Astro(#[from] AstroError),
    #[error("INPUT_HASH_FAILED: {0}")]
    InputHash(String),
}

pub struct TrajectorySolver {
    catalog: Arc<Catalog>,
    ephemeris: EphemerisService,
}

impl TrajectorySolver {
    pub fn new(catalog: Arc<Catalog>) -> Self {
        Self {
            ephemeris: EphemerisService::new(Arc::clone(&catalog)),
            catalog,
        }
    }

    pub fn bundled() -> Result<Self, TrajectoryError> {
        Ok(Self::new(Arc::new(Catalog::bundled()?)))
    }

    pub fn search(
        &self,
        request: &TransferRequest,
        blueprint: &ShipBlueprint,
        vessel: &VesselState,
        cancellation: &CancellationToken,
    ) -> Result<TransferSearchReport, TrajectoryError> {
        self.search_with_progress(request, blueprint, vessel, cancellation, |_| {})
    }

    pub fn search_with_progress<F>(
        &self,
        request: &TransferRequest,
        blueprint: &ShipBlueprint,
        vessel: &VesselState,
        cancellation: &CancellationToken,
        mut on_progress: F,
    ) -> Result<TransferSearchReport, TrajectoryError>
    where
        F: FnMut(SearchProgress),
    {
        validate_request(&self.catalog, request, blueprint, vessel)?;
        let input_hash = transfer_input_hash(request, blueprint, vessel)?;
        let planned = request
            .options
            .departure_samples
            .saturating_mul(request.options.duration_samples);
        let budget = request.options.maximum_evaluations.min(planned);
        let mut solutions = Vec::new();
        let mut failures = Vec::new();
        let departures = sample_instants(
            request.departure_window.earliest,
            request.departure_window.latest,
            request.options.departure_samples,
        )?;
        let durations = sample_values(
            request.duration_window.minimum_s,
            request.duration_window.maximum_s,
            request.options.duration_samples,
        );
        let mut evaluated = 0_u32;
        let mut status = SearchStatus::Completed;

        'search: for departure in departures {
            for duration_s in &durations {
                if cancellation.is_cancelled() {
                    status = SearchStatus::Cancelled;
                    break 'search;
                }
                if evaluated >= budget {
                    status = SearchStatus::PartialBudgetExhausted;
                    break 'search;
                }
                evaluated += 1;
                match self.solve_candidate(
                    request,
                    blueprint,
                    vessel,
                    departure,
                    *duration_s,
                    &input_hash,
                ) {
                    Ok(solution) => solutions.push(solution),
                    Err(failure) => failures.push(failure),
                }
                on_progress(SearchProgress {
                    evaluated,
                    planned,
                    executable_solutions: solutions.len(),
                    status,
                });
            }
        }

        let termination_reason = match status {
            SearchStatus::Cancelled => TerminationReason::Cancelled,
            SearchStatus::PartialBudgetExhausted => TerminationReason::SearchBudgetExhausted,
            SearchStatus::Completed if !solutions.is_empty() => TerminationReason::Converged,
            SearchStatus::Completed => dominant_failure_reason(&failures),
        };
        on_progress(SearchProgress {
            evaluated,
            planned,
            executable_solutions: solutions.len(),
            status,
        });
        Ok(TransferSearchReport {
            input_hash,
            solutions,
            failures,
            evaluated,
            planned,
            status,
            termination_reason,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn solve_candidate(
        &self,
        request: &TransferRequest,
        blueprint: &ShipBlueprint,
        vessel: &VesselState,
        departure: TdbInstant,
        duration_s: f64,
        input_hash: &str,
    ) -> Result<TransferSolution, CandidateFailure> {
        let arrival = add_seconds(departure, duration_s).map_err(|message| CandidateFailure {
            departure: Some(departure),
            duration_s: Some(duration_s),
            kind: CandidateFailureKind::InvalidRequest,
            message,
            constraints: Vec::new(),
        })?;
        let origin = self
            .ephemeris
            .state(&request.origin_id, departure)
            .map_err(|error| candidate_astro_failure(departure, duration_s, error))?;
        let destination = self
            .ephemeris
            .state(&request.destination_id, arrival)
            .map_err(|error| candidate_astro_failure(departure, duration_s, error))?;
        let sun_id = StableId::new("sun").expect("static id is valid");
        let central_mu = self
            .catalog
            .body(&sun_id)
            .map_err(|error| candidate_astro_failure(departure, duration_s, error))?
            .mass_kg
            .value()
            * GRAVITATIONAL_CONSTANT_M3_KG_S2;
        let lambert = solve_lambert_universal(
            origin.state.position_m,
            destination.state.position_m,
            duration_s,
            central_mu,
            request.options.direction,
        )
        .map_err(|error| candidate_lambert_failure(departure, duration_s, error))?;

        let departure_v_infinity = sub(lambert.departure_velocity_mps, origin.state.velocity_mps);
        let arrival_v_infinity = sub(lambert.arrival_velocity_mps, destination.state.velocity_mps);
        let departure_delta_v = local_parking_delta_v(
            self.catalog
                .body(&request.origin_id)
                .map_err(|error| candidate_astro_failure(departure, duration_s, error))?,
            norm(departure_v_infinity),
        );
        let arrival_delta_v = match request.arrival_condition {
            ArrivalCondition::Rendezvous => local_parking_delta_v(
                self.catalog
                    .body(&request.destination_id)
                    .map_err(|error| candidate_astro_failure(departure, duration_s, error))?,
                norm(arrival_v_infinity),
            ),
            ArrivalCondition::Flyby => 0.0,
        };

        let mut working_vessel = vessel.clone();
        working_vessel.payload_mass_kg = request.payload_mass_kg;
        working_vessel.payload_volume_m3 = request.payload_volume_m3;
        let initial_propellant = working_vessel.propellant_kg.value();
        let initial_fusion_fuel = working_vessel.fusion_fuel_kg.value();
        let initial_reactor_life = working_vessel.reactor_full_power_used_s.value();
        let initial_engine_life = working_vessel.engine_full_power_used_s.value();

        let departure_plan = plan_finite_burn(
            blueprint,
            &working_vessel,
            departure_delta_v,
            unit(departure_v_infinity).unwrap_or([1.0, 0.0, 0.0]),
            &request.reserve_policy,
        )
        .map_err(|constraints| CandidateFailure {
            departure: Some(departure),
            duration_s: Some(duration_s),
            kind: CandidateFailureKind::ConstraintViolation,
            message: "departure finite-thrust maneuver violates vessel constraints".into(),
            constraints,
        })?;
        working_vessel = departure_plan.final_vessel.clone();
        reset_after_coast(&mut working_vessel);
        let arrival_plan = plan_finite_burn(
            blueprint,
            &working_vessel,
            arrival_delta_v,
            unit(sub(
                destination.state.velocity_mps,
                lambert.arrival_velocity_mps,
            ))
            .unwrap_or([1.0, 0.0, 0.0]),
            &request.reserve_policy,
        )
        .map_err(|constraints| CandidateFailure {
            departure: Some(departure),
            duration_s: Some(duration_s),
            kind: CandidateFailureKind::ConstraintViolation,
            message: "arrival finite-thrust maneuver violates vessel constraints".into(),
            constraints,
        })?;
        working_vessel = arrival_plan.final_vessel.clone();
        let maneuver_elapsed = departure_plan.elapsed_duration_s + arrival_plan.elapsed_duration_s;
        if maneuver_elapsed >= duration_s * 0.5 {
            return Err(CandidateFailure {
                departure: Some(departure),
                duration_s: Some(duration_s),
                kind: CandidateFailureKind::ConstraintViolation,
                message: "finite-thrust maneuvers consume at least half of the transfer duration"
                    .into(),
                constraints: Vec::new(),
            });
        }

        let verification = integrate_two_body_adaptive(
            CartesianState {
                position_m: lambert.departure_position_m,
                velocity_mps: lambert.departure_velocity_mps,
            },
            duration_s,
            central_mu,
            request.options.integrator,
        )
        .map_err(|error| CandidateFailure {
            departure: Some(departure),
            duration_s: Some(duration_s),
            kind: CandidateFailureKind::NumericalNonConvergence,
            message: error.to_string(),
            constraints: Vec::new(),
        })?;
        let position_error = norm(sub(
            verification.final_state.position_m,
            lambert.arrival_position_m,
        ));
        let velocity_error = norm(sub(
            verification.final_state.velocity_mps,
            lambert.arrival_velocity_mps,
        ));
        if position_error > request.options.verification_position_tolerance_m
            || velocity_error > request.options.verification_velocity_tolerance_mps
        {
            return Err(CandidateFailure {
                departure: Some(departure),
                duration_s: Some(duration_s),
                kind: CandidateFailureKind::NumericalNonConvergence,
                message: format!(
                    "independent integration error position={position_error:.3} m velocity={velocity_error:.6} m/s"
                ),
                constraints: Vec::new(),
            });
        }

        let departure_burn_end = add_seconds(departure, departure_plan.elapsed_duration_s)
            .map_err(|message| CandidateFailure {
                departure: Some(departure),
                duration_s: Some(duration_s),
                kind: CandidateFailureKind::InvalidRequest,
                message,
                constraints: Vec::new(),
            })?;
        let arrival_burn_start =
            add_seconds(arrival, -arrival_plan.elapsed_duration_s).map_err(|message| {
                CandidateFailure {
                    departure: Some(departure),
                    duration_s: Some(duration_s),
                    kind: CandidateFailureKind::InvalidRequest,
                    message,
                    constraints: Vec::new(),
                }
            })?;
        let mut segments = Vec::new();
        if departure_plan.target_delta_v_mps > 0.0 {
            segments.push(departure_plan.to_segment("departure", departure, departure_burn_end));
        }
        segments.push(TrajectorySegment::Coast {
            start: departure_burn_end,
            end: arrival_burn_start,
            start_state: StateVector::new(
                origin.state.frame_id.clone(),
                departure_burn_end,
                lambert.departure_position_m,
                lambert.departure_velocity_mps,
            )
            .map_err(|error| candidate_astro_failure(departure, duration_s, error))?,
            end_state: StateVector::new(
                destination.state.frame_id.clone(),
                arrival_burn_start,
                lambert.arrival_position_m,
                lambert.arrival_velocity_mps,
            )
            .map_err(|error| candidate_astro_failure(departure, duration_s, error))?,
        });
        if arrival_plan.target_delta_v_mps > 0.0 {
            segments.push(arrival_plan.to_segment("arrival", arrival_burn_start, arrival));
        }
        segments.push(TrajectorySegment::Approach {
            at: arrival,
            destination_id: request.destination_id.clone(),
            planned_position_error_m: position_error,
            planned_velocity_error_mps: velocity_error,
        });

        let patched_conic_events = vec![
            PatchedConicEvent {
                kind: PatchedConicEventKind::DepartureEscape,
                body_id: Some(request.origin_id.clone()),
                epoch: departure,
                hyperbolic_excess_velocity_mps: norm(departure_v_infinity),
                local_maneuver_delta_v_mps: departure_delta_v,
            },
            PatchedConicEvent {
                kind: PatchedConicEventKind::HeliocentricTransfer,
                body_id: None,
                epoch: departure_burn_end,
                hyperbolic_excess_velocity_mps: 0.0,
                local_maneuver_delta_v_mps: 0.0,
            },
            PatchedConicEvent {
                kind: PatchedConicEventKind::ArrivalApproach,
                body_id: Some(request.destination_id.clone()),
                epoch: arrival,
                hyperbolic_excess_velocity_mps: norm(arrival_v_infinity),
                local_maneuver_delta_v_mps: arrival_delta_v,
            },
        ];
        let peak_power = departure_plan.input_power_w.max(arrival_plan.input_power_w);
        let peak_heat = departure_plan
            .peak_waste_heat_w
            .max(arrival_plan.peak_waste_heat_w);
        let candidate_hash = blake3::hash(
            format!(
                "{input_hash}:{}:{duration_s:.6}",
                departure.micros_since_j2000()
            )
            .as_bytes(),
        )
        .to_hex()
        .to_string();
        let solution_id = StableId::new(format!("solution:{}", &candidate_hash[..24]))
            .expect("hex digest creates a valid stable id");
        let destination_body = self
            .catalog
            .body(&request.destination_id)
            .map_err(|error| candidate_astro_failure(departure, duration_s, error))?;
        let commercial = destination_body.development_status == DevelopmentStatus::CommercialOpen;

        Ok(TransferSolution {
            id: solution_id,
            origin_id: request.origin_id.clone(),
            destination_id: request.destination_id.clone(),
            departure,
            arrival,
            time_of_flight_s: duration_s,
            payload_mass_kg: request.payload_mass_kg.value(),
            lambert_arc: lambert.clone(),
            patched_conic_events,
            segments,
            propellant_consumed_kg: initial_propellant - working_vessel.propellant_kg.value(),
            fusion_fuel_consumed_kg: initial_fusion_fuel - working_vessel.fusion_fuel_kg.value(),
            peak_power_w: peak_power,
            peak_waste_heat_w: peak_heat,
            reactor_lifetime_used_s: working_vessel.reactor_full_power_used_s.value()
                - initial_reactor_life,
            engine_lifetime_used_s: working_vessel.engine_full_power_used_s.value()
                - initial_engine_life,
            estimated_cost_credits: (initial_propellant - working_vessel.propellant_kg.value())
                * 2.0
                + (initial_fusion_fuel - working_vessel.fusion_fuel_kg.value()) * 1_000.0
                + ((working_vessel.reactor_full_power_used_s.value() - initial_reactor_life)
                    + (working_vessel.engine_full_power_used_s.value() - initial_engine_life))
                    * 0.05,
            margins: FeasibilityMargins {
                position_error_m: position_error,
                velocity_error_mps: velocity_error,
                propellant_remaining_kg: working_vessel.propellant_kg.value(),
                fusion_fuel_remaining_kg: working_vessel.fusion_fuel_kg.value(),
                reactor_lifetime_remaining_s: blueprint
                    .lifetime
                    .reactor_full_power_lifetime_s
                    .value()
                    - working_vessel.reactor_full_power_used_s.value(),
                engine_lifetime_remaining_s: blueprint
                    .lifetime
                    .engine_full_power_lifetime_s
                    .value()
                    - working_vessel.engine_full_power_used_s.value(),
            },
            destination_services: DestinationServices {
                market: commercial,
                propellant_supply: commercial,
                repair: commercial,
            },
            validation_level: ValidationLevel::Executable,
            metadata: SolverMetadata {
                input_hash: input_hash.into(),
                solver_version: SOLVER_VERSION.into(),
                lambert_iterations: lambert.iterations,
                integrator_accepted_steps: verification.accepted_steps,
                integrator_rejected_steps: verification.rejected_steps,
                position_tolerance_m: request.options.verification_position_tolerance_m,
                velocity_tolerance_mps: request.options.verification_velocity_tolerance_mps,
                termination_reason: TerminationReason::Converged,
            },
        })
    }
}

#[derive(Debug, Clone)]
struct FiniteBurnPlan {
    final_vessel: VesselState,
    direction: Vector3,
    target_delta_v_mps: f64,
    thrust_n: f64,
    exhaust_velocity_mps: f64,
    input_power_w: f64,
    powered_duration_s: f64,
    elapsed_duration_s: f64,
    chunk_count: u32,
    initial_mass_kg: f64,
    final_mass_kg: f64,
    peak_waste_heat_w: f64,
}

impl FiniteBurnPlan {
    fn zero(vessel: &VesselState, blueprint: &ShipBlueprint, direction: Vector3) -> Self {
        let mass = vessel.total_mass_kg(blueprint);
        Self {
            final_vessel: vessel.clone(),
            direction,
            target_delta_v_mps: 0.0,
            thrust_n: 0.0,
            exhaust_velocity_mps: 0.0,
            input_power_w: 0.0,
            powered_duration_s: 0.0,
            elapsed_duration_s: 0.0,
            chunk_count: 0,
            initial_mass_kg: mass,
            final_mass_kg: mass,
            peak_waste_heat_w: 0.0,
        }
    }

    fn to_segment(&self, phase: &str, start: TdbInstant, end: TdbInstant) -> TrajectorySegment {
        TrajectorySegment::FiniteBurn {
            phase: phase.into(),
            start,
            end,
            direction: self.direction,
            target_delta_v_mps: self.target_delta_v_mps,
            thrust_n: self.thrust_n,
            effective_exhaust_velocity_mps: self.exhaust_velocity_mps,
            input_power_w: self.input_power_w,
            powered_duration_s: self.powered_duration_s,
            elapsed_duration_s: self.elapsed_duration_s,
            chunk_count: self.chunk_count,
            initial_mass_kg: self.initial_mass_kg,
            final_mass_kg: self.final_mass_kg,
            peak_waste_heat_w: self.peak_waste_heat_w,
        }
    }
}

fn plan_finite_burn(
    blueprint: &ShipBlueprint,
    vessel: &VesselState,
    delta_v_mps: f64,
    direction: Vector3,
    reserves: &ReservePolicy,
) -> Result<FiniteBurnPlan, Vec<ConstraintViolation>> {
    if delta_v_mps <= 1e-9 {
        return Ok(FiniteBurnPlan::zero(vessel, blueprint, direction));
    }
    let minimum_exhaust = blueprint
        .propulsion
        .min_effective_exhaust_velocity_mps
        .value();
    let maximum_exhaust = blueprint
        .propulsion
        .max_effective_exhaust_velocity_mps
        .value();
    let maximum_power = blueprint.propulsion.max_input_power_w.value();
    let mut last_violations = Vec::new();

    for exhaust_fraction in [1.0, 0.8, 0.6, 0.4, 0.2] {
        let exhaust_velocity =
            minimum_exhaust + (maximum_exhaust - minimum_exhaust) * exhaust_fraction;
        for power_fraction in [1.0, 0.75, 0.5, 0.25] {
            let power = maximum_power * power_fraction;
            let jet_power = blueprint.propulsion.electrical_to_jet_efficiency.value() * power;
            let thrust = 2.0 * jet_power / exhaust_velocity;
            let mass_flow = thrust / exhaust_velocity;
            let initial_mass = vessel.total_mass_kg(blueprint);
            let required_propellant =
                initial_mass * (1.0 - (-delta_v_mps / exhaust_velocity).exp());
            let powered_duration = required_propellant / mass_flow;
            let maximum_chunk = blueprint
                .lifetime
                .max_continuous_burn_s
                .value()
                .min(6.0 * 3_600.0);
            let chunk_count = (powered_duration / maximum_chunk).ceil().max(1.0) as u32;
            let chunk_duration = powered_duration / f64::from(chunk_count);
            let mut candidate = vessel.clone();
            let mut elapsed = 0.0;
            let mut peak_heat = 0.0_f64;
            let mut first_outcome: Option<BurnOutcome> = None;
            let mut feasible = true;
            for chunk_index in 0..chunk_count {
                let burn_request = BurnRequest {
                    duration_s: DurationSeconds::new(chunk_duration)
                        .expect("computed duration is finite and non-negative"),
                    propulsion_input_power_w: PowerWatts::new(power)
                        .expect("blueprint power is finite and non-negative"),
                    effective_exhaust_velocity_mps: VelocityMetersPerSecond::new(exhaust_velocity)
                        .expect("blueprint exhaust velocity is finite and positive"),
                };
                let assessment = apply_burn(blueprint, &mut candidate, &burn_request, reserves);
                let Some(outcome) = assessment.outcome else {
                    last_violations = assessment.violations;
                    feasible = false;
                    break;
                };
                peak_heat = peak_heat.max(outcome.peak_waste_heat_w);
                first_outcome.get_or_insert_with(|| outcome.clone());
                elapsed += chunk_duration;
                if chunk_index + 1 < chunk_count {
                    let cooldown = (outcome.final_thermal_buffer_j
                        / blueprint.thermal.continuous_heat_rejection_w.value())
                    .max(chunk_duration * 0.1);
                    elapsed += cooldown;
                    reset_after_coast(&mut candidate);
                }
            }
            if feasible {
                let first = first_outcome.expect("positive burn has at least one chunk");
                return Ok(FiniteBurnPlan {
                    final_vessel: candidate.clone(),
                    direction,
                    target_delta_v_mps: delta_v_mps,
                    thrust_n: first.thrust_n,
                    exhaust_velocity_mps: exhaust_velocity,
                    input_power_w: power,
                    powered_duration_s: powered_duration,
                    elapsed_duration_s: elapsed,
                    chunk_count,
                    initial_mass_kg: initial_mass,
                    final_mass_kg: candidate.total_mass_kg(blueprint),
                    peak_waste_heat_w: peak_heat,
                });
            }
        }
    }
    Err(last_violations)
}

fn reset_after_coast(vessel: &mut VesselState) {
    vessel.current_continuous_burn_s = DurationSeconds::new(0.0).expect("zero duration is valid");
    vessel.thermal_buffer_j = EnergyJoules::new(0.0).expect("zero energy is valid");
}

pub fn local_parking_delta_v(body: &sim_astro::CelestialBody, v_infinity_mps: f64) -> f64 {
    if body.parent_id.is_none() {
        return v_infinity_mps;
    }
    let parking_altitude = (body.mean_radius_m.value() * 0.05).max(100_000.0);
    let parking_radius = body.mean_radius_m.value() + parking_altitude;
    let body_mu = GRAVITATIONAL_CONSTANT_M3_KG_S2 * body.mass_kg.value();
    let circular_speed = (body_mu / parking_radius).sqrt();
    let hyperbolic_periapsis_speed =
        (v_infinity_mps * v_infinity_mps + 2.0 * body_mu / parking_radius).sqrt();
    (hyperbolic_periapsis_speed - circular_speed).max(0.0)
}

pub fn transfer_input_hash(
    request: &TransferRequest,
    blueprint: &ShipBlueprint,
    vessel: &VesselState,
) -> Result<String, TrajectoryError> {
    let bytes = serde_json::to_vec(&(request, blueprint, vessel))
        .map_err(|error| TrajectoryError::InputHash(error.to_string()))?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn validate_request(
    catalog: &Catalog,
    request: &TransferRequest,
    blueprint: &ShipBlueprint,
    vessel: &VesselState,
) -> Result<(), TrajectoryError> {
    catalog.body(&request.origin_id)?;
    catalog.body(&request.destination_id)?;
    if request.origin_id == request.destination_id {
        return Err(TrajectoryError::InvalidRequest(
            "origin and destination must differ".into(),
        ));
    }
    if request.departure_window.latest < request.departure_window.earliest {
        return Err(TrajectoryError::InvalidRequest(
            "departure window is reversed".into(),
        ));
    }
    if !request.duration_window.minimum_s.is_finite()
        || !request.duration_window.maximum_s.is_finite()
        || request.duration_window.minimum_s <= 0.0
        || request.duration_window.maximum_s < request.duration_window.minimum_s
    {
        return Err(TrajectoryError::InvalidRequest(
            "duration window must be finite, positive, and ordered".into(),
        ));
    }
    if request.options.departure_samples == 0
        || request.options.duration_samples == 0
        || request.options.maximum_evaluations == 0
    {
        return Err(TrajectoryError::InvalidRequest(
            "sample counts and search budget must be positive".into(),
        ));
    }
    blueprint
        .validate()
        .map_err(|error| TrajectoryError::InvalidRequest(error.to_string()))?;
    if request.vessel_id != vessel.id {
        return Err(TrajectoryError::InvalidRequest(
            "request vessel id does not match vessel state".into(),
        ));
    }
    let mut state = vessel.clone();
    state.payload_mass_kg = request.payload_mass_kg;
    state.payload_volume_m3 = request.payload_volume_m3;
    let violations = state.validate(blueprint);
    if !violations.is_empty() {
        return Err(TrajectoryError::InvalidRequest(format!(
            "vessel state violates {} engineering constraints",
            violations.len()
        )));
    }
    if request.options.verification_position_tolerance_m <= 0.0
        || request.options.verification_velocity_tolerance_mps <= 0.0
    {
        return Err(TrajectoryError::InvalidRequest(
            "verification tolerances must be positive".into(),
        ));
    }
    Ok(())
}

fn sample_instants(
    earliest: TdbInstant,
    latest: TdbInstant,
    count: u32,
) -> Result<Vec<TdbInstant>, TrajectoryError> {
    if count == 1 {
        return Ok(vec![earliest]);
    }
    let span = latest
        .micros_since_j2000()
        .checked_sub(earliest.micros_since_j2000())
        .ok_or_else(|| TrajectoryError::InvalidRequest("departure window overflow".into()))?;
    (0..count)
        .map(|index| {
            let offset = i128::from(span) * i128::from(index) / i128::from(count - 1);
            let micros = i128::from(earliest.micros_since_j2000()) + offset;
            i64::try_from(micros)
                .map(TdbInstant::from_micros_since_j2000)
                .map_err(|_| TrajectoryError::InvalidRequest("sample time overflow".into()))
        })
        .collect()
}

fn sample_values(minimum: f64, maximum: f64, count: u32) -> Vec<f64> {
    if count == 1 {
        return vec![minimum];
    }
    (0..count)
        .map(|index| minimum + (maximum - minimum) * f64::from(index) / f64::from(count - 1))
        .collect()
}

fn add_seconds(instant: TdbInstant, seconds: f64) -> Result<TdbInstant, String> {
    let micros = seconds * MICROS_PER_SECOND as f64;
    if !micros.is_finite() || micros < i64::MIN as f64 || micros > i64::MAX as f64 {
        return Err("time offset is out of range".into());
    }
    instant
        .checked_add_micros(micros.round() as i64)
        .map_err(|error| error.to_string())
}

fn candidate_astro_failure(
    departure: TdbInstant,
    duration_s: f64,
    error: AstroError,
) -> CandidateFailure {
    CandidateFailure {
        departure: Some(departure),
        duration_s: Some(duration_s),
        kind: CandidateFailureKind::InvalidRequest,
        message: error.to_string(),
        constraints: Vec::new(),
    }
}

fn candidate_lambert_failure(
    departure: TdbInstant,
    duration_s: f64,
    error: LambertError,
) -> CandidateFailure {
    let kind = match error {
        LambertError::NoPhysicalSolution(_) => CandidateFailureKind::NoPhysicalSolution,
        LambertError::NumericalNonConvergence(_) => CandidateFailureKind::NumericalNonConvergence,
    };
    CandidateFailure {
        departure: Some(departure),
        duration_s: Some(duration_s),
        kind,
        message: error.to_string(),
        constraints: Vec::new(),
    }
}

fn dominant_failure_reason(failures: &[CandidateFailure]) -> TerminationReason {
    if failures
        .iter()
        .any(|failure| failure.kind == CandidateFailureKind::ConstraintViolation)
    {
        TerminationReason::ConstraintViolation
    } else if failures
        .iter()
        .any(|failure| failure.kind == CandidateFailureKind::NumericalNonConvergence)
    {
        TerminationReason::NumericalNonConvergence
    } else {
        TerminationReason::NoPhysicalSolution
    }
}

pub fn standard_test_vessel(
    blueprint_id: &str,
) -> Result<(ShipBlueprint, VesselState), TrajectoryError> {
    let engineering = EngineeringCatalog::bundled()
        .map_err(|error| TrajectoryError::InvalidRequest(error.to_string()))?;
    let id = StableId::new(blueprint_id)
        .map_err(|error| TrajectoryError::InvalidRequest(error.to_string()))?;
    let blueprint = engineering
        .blueprint(&id, 1)
        .ok_or_else(|| TrajectoryError::InvalidRequest("test blueprint not found".into()))?
        .clone();
    let vessel = VesselState {
        id: StableId::new("vessel:trajectory-test")
            .map_err(|error| TrajectoryError::InvalidRequest(error.to_string()))?,
        blueprint_id: blueprint.id.clone(),
        blueprint_revision: blueprint.revision,
        payload_mass_kg: MassKilograms::new(0.0)
            .map_err(|error| TrajectoryError::InvalidRequest(error.to_string()))?,
        payload_volume_m3: VolumeCubicMeters::new(0.0)
            .map_err(|error| TrajectoryError::InvalidRequest(error.to_string()))?,
        fusion_fuel_kg: blueprint.fusion_fuel_capacity_kg,
        propellant_kg: blueprint.propellant_capacity_kg,
        thermal_buffer_j: EnergyJoules::new(0.0)
            .map_err(|error| TrajectoryError::InvalidRequest(error.to_string()))?,
        current_continuous_burn_s: DurationSeconds::new(0.0)
            .map_err(|error| TrajectoryError::InvalidRequest(error.to_string()))?,
        reactor_full_power_used_s: DurationSeconds::new(0.0)
            .map_err(|error| TrajectoryError::InvalidRequest(error.to_string()))?,
        engine_full_power_used_s: DurationSeconds::new(0.0)
            .map_err(|error| TrajectoryError::InvalidRequest(error.to_string()))?,
    };
    Ok((blueprint, vessel))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_time::{CalendarDateTime, MICROS_PER_DAY};

    fn instant() -> TdbInstant {
        TdbInstant::from_utc(CalendarDateTime::new(2160, 1, 1, 0, 0, 0, 0).unwrap()).unwrap()
    }

    fn request(destination: &str) -> TransferRequest {
        let departure = instant();
        TransferRequest {
            origin_id: StableId::new("earth").unwrap(),
            destination_id: StableId::new(destination).unwrap(),
            departure_window: TimeWindow {
                earliest: departure,
                latest: departure.checked_add_micros(10 * MICROS_PER_DAY).unwrap(),
            },
            duration_window: DurationWindow {
                minimum_s: 3.0 * 86_400.0,
                maximum_s: 30.0 * 86_400.0,
            },
            vessel_id: StableId::new("vessel:trajectory-test").unwrap(),
            payload_mass_kg: MassKilograms::new(100.0).unwrap(),
            payload_volume_m3: VolumeCubicMeters::new(1.0).unwrap(),
            reserve_policy: ReservePolicy::zero(),
            arrival_condition: ArrivalCondition::Flyby,
            options: SolverOptions {
                departure_samples: 2,
                duration_samples: 2,
                maximum_evaluations: 4,
                ..SolverOptions::default()
            },
        }
    }

    #[test]
    fn patched_conic_escape_matches_energy_equation() {
        let catalog = Catalog::bundled().unwrap();
        let earth = catalog.body(&StableId::new("earth").unwrap()).unwrap();
        let delta_v = local_parking_delta_v(earth, 3_000.0);
        assert!(
            (delta_v - 3_602.15).abs() < 0.05,
            "parking delta-v {delta_v}"
        );
    }

    #[test]
    fn search_budget_and_cancellation_are_structured() {
        let solver = TrajectorySolver::bundled().unwrap();
        let (blueprint, vessel) = standard_test_vessel("ship:autonomous-surveyor").unwrap();
        let mut limited = request("moon");
        limited.options.maximum_evaluations = 1;
        let report = solver
            .search(&limited, &blueprint, &vessel, &CancellationToken::default())
            .unwrap();
        assert_eq!(report.status, SearchStatus::PartialBudgetExhausted);
        assert_eq!(
            report.termination_reason,
            TerminationReason::SearchBudgetExhausted
        );

        let cancellation = CancellationToken::default();
        cancellation.cancel();
        let report = solver
            .search(&request("moon"), &blueprint, &vessel, &cancellation)
            .unwrap();
        assert_eq!(report.status, SearchStatus::Cancelled);
        assert_eq!(report.evaluated, 0);
    }

    #[test]
    fn every_catalog_body_is_accepted_as_a_request_target() {
        let solver = TrajectorySolver::bundled().unwrap();
        let catalog = Catalog::bundled().unwrap();
        let (blueprint, vessel) = standard_test_vessel("ship:autonomous-surveyor").unwrap();
        for body in catalog
            .bodies()
            .iter()
            .filter(|body| body.id.as_str() != "earth")
        {
            let mut target_request = request(body.id.as_str());
            target_request.options.departure_samples = 1;
            target_request.options.duration_samples = 1;
            target_request.options.maximum_evaluations = 1;
            let report = solver.search(
                &target_request,
                &blueprint,
                &vessel,
                &CancellationToken::default(),
            );
            assert!(
                report.is_ok(),
                "target {} was rejected: {report:?}",
                body.id
            );
        }
    }

    #[test]
    fn executable_solution_records_reproduction_and_independent_verification() {
        let solver = TrajectorySolver::bundled().unwrap();
        let (blueprint, vessel) = standard_test_vessel("ship:lunar-courier").unwrap();
        let mut transfer_request = request("moon");
        transfer_request.arrival_condition = ArrivalCondition::Rendezvous;
        let report = solver
            .search(
                &transfer_request,
                &blueprint,
                &vessel,
                &CancellationToken::default(),
            )
            .unwrap();
        let solution = report
            .solutions
            .first()
            .unwrap_or_else(|| panic!("no solution; failures={:#?}", report.failures));
        assert_eq!(solution.validation_level, ValidationLevel::Executable);
        assert_eq!(solution.metadata.solver_version, SOLVER_VERSION);
        assert_eq!(solution.metadata.input_hash, report.input_hash);
        assert!(solution.metadata.integrator_accepted_steps > 0);
        assert!(!solution.destination_services.market);
        assert!(solution.propellant_consumed_kg >= 0.0);
    }
}
