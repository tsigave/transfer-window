use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use sim_app::{ScheduleVoyageCommand, SimulationApp, VoyageStatus};
use sim_astro::{Catalog, EphemerisService};
use sim_engineering::{EngineeringCatalog, MassKilograms, ReservePolicy, VolumeCubicMeters};
use sim_time::{CalendarDateTime, StableId, TdbInstant, MICROS_PER_DAY};
use sim_trajectory::{
    local_parking_delta_v, solve_lambert_universal, standard_test_vessel, ArrivalCondition,
    CancellationToken, DurationWindow, SolverOptions, TimeWindow, TrajectorySolver,
    TransferDirection, TransferRequest, ValidationLevel,
};
use std::sync::Arc;

#[derive(Debug, Parser)]
#[command(
    name = "sim-tools",
    version,
    about = "Transfer Window simulation audit tools"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Catalog {
        #[command(subcommand)]
        command: CatalogCommand,
    },
    Ephemeris {
        #[command(subcommand)]
        command: EphemerisCommand,
    },
    Engineering {
        #[command(subcommand)]
        command: EngineeringCommand,
    },
    Replay {
        #[command(subcommand)]
        command: ReplayCommand,
    },
    Trajectory {
        #[command(subcommand)]
        command: TrajectoryCommand,
    },
}

#[derive(Debug, Subcommand)]
enum CatalogCommand {
    Audit,
}

#[derive(Debug, Subcommand)]
enum EphemerisCommand {
    Verify(VerifyArgs),
}

#[derive(Debug, Subcommand)]
enum EngineeringCommand {
    Audit,
}

#[derive(Debug, Args)]
struct VerifyArgs {
    #[arg(long, default_value_t = 200)]
    span_years: u32,
}

#[derive(Debug, Subcommand)]
enum ReplayCommand {
    EmptyWorld(ReplayArgs),
    Voyage(ReplayArgs),
}

#[derive(Debug, Subcommand)]
enum TrajectoryCommand {
    Golden,
    CatalogSmoke,
}

#[derive(Debug, Args)]
struct ReplayArgs {
    #[arg(long, value_delimiter = ',', default_value = "0,1,100,10000")]
    rates: Vec<u32>,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Catalog {
            command: CatalogCommand::Audit,
        } => catalog_audit(),
        Command::Ephemeris {
            command: EphemerisCommand::Verify(args),
        } => ephemeris_verify(args.span_years),
        Command::Engineering {
            command: EngineeringCommand::Audit,
        } => engineering_audit(),
        Command::Replay {
            command: ReplayCommand::EmptyWorld(args),
        } => replay_empty_world(&args.rates),
        Command::Replay {
            command: ReplayCommand::Voyage(args),
        } => replay_voyage(&args.rates),
        Command::Trajectory {
            command: TrajectoryCommand::Golden,
        } => trajectory_golden(),
        Command::Trajectory {
            command: TrajectoryCommand::CatalogSmoke,
        } => trajectory_catalog_smoke(),
    }
}

fn trajectory_golden() -> Result<()> {
    let lambert = solve_lambert_universal(
        [5_000_000.0, 10_000_000.0, 2_100_000.0],
        [-14_600_000.0, 2_500_000.0, 7_000_000.0],
        3_600.0,
        3.986_004_418e14,
        TransferDirection::ShortWay,
    )?;
    let expected_departure = [-5_992.495, 1_925.367, 3_245.638];
    let lambert_error = norm(subtract(lambert.departure_velocity_mps, expected_departure));
    if lambert_error > 0.04 {
        bail!("Lambert golden velocity error {lambert_error:.6} m/s");
    }

    let catalog = Catalog::bundled()?;
    let earth = catalog.body(&StableId::new("earth")?)?;
    let escape_delta_v = local_parking_delta_v(earth, 3_000.0);
    if (escape_delta_v - 3_602.15).abs() > 0.05 {
        bail!("patched-conics escape error: {escape_delta_v:.6} m/s");
    }

    let solver = TrajectorySolver::bundled()?;
    let (blueprint, vessel) = standard_test_vessel("ship:lunar-courier")?;
    let mut request = trajectory_request("earth", "moon", &vessel.id, 3.0, 30.0, 2, 4)?;
    request.arrival_condition = ArrivalCondition::Rendezvous;
    let report = solver.search(&request, &blueprint, &vessel, &CancellationToken::default())?;
    let solution = report
        .solutions
        .first()
        .ok_or_else(|| anyhow::anyhow!("finite-thrust golden produced no executable solution"))?;
    if solution.validation_level != ValidationLevel::Executable
        || solution.propellant_consumed_kg < 0.0
        || solution.metadata.integrator_accepted_steps == 0
    {
        bail!("finite-thrust or independent verification golden failed");
    }

    println!("lambert_velocity_error_mps={lambert_error:.9}");
    println!("patched_conics_escape_delta_v_mps={escape_delta_v:.6}");
    println!("finite_thrust_solution_id={}", solution.id);
    println!(
        "finite_thrust_propellant_kg={:.6}",
        solution.propellant_consumed_kg
    );
    println!(
        "verification_position_error_m={:.6}",
        solution.margins.position_error_m
    );
    println!("trajectory golden: PASS");
    Ok(())
}

fn trajectory_catalog_smoke() -> Result<()> {
    let catalog = Catalog::bundled()?;
    let solver = TrajectorySolver::bundled()?;
    let (blueprint, vessel) = standard_test_vessel("ship:autonomous-surveyor")?;
    let cancellation = CancellationToken::default();
    let mut executable = 0_usize;
    let mut structured_infeasible = 0_usize;
    for body in catalog.bodies() {
        let (origin, duration_days) = if body.id.as_str() == "earth" {
            ("moon", 30.0)
        } else if body
            .parent_id
            .as_ref()
            .is_some_and(|parent| parent.as_str() != "sun")
        {
            ("earth", 120.0)
        } else {
            ("earth", 1_200.0)
        };
        let request = trajectory_request(
            origin,
            body.id.as_str(),
            &vessel.id,
            duration_days * 0.5,
            duration_days,
            1,
            1,
        )?;
        let report = solver.search(&request, &blueprint, &vessel, &cancellation)?;
        if report.solutions.is_empty() {
            if report.failures.is_empty() {
                bail!(
                    "target {} returned neither result nor structured reason",
                    body.id
                );
            }
            structured_infeasible += 1;
            println!(
                "target={:<12} result={:?} detail={}",
                body.id, report.termination_reason, report.failures[0].message
            );
        } else {
            executable += 1;
            println!(
                "target={:<12} result=EXECUTABLE solution={}",
                body.id, report.solutions[0].id
            );
        }
    }
    println!("catalog_bodies={}", catalog.bodies().len());
    println!("executable={executable}");
    println!("structured_infeasible={structured_infeasible}");
    println!("trajectory catalog smoke: PASS");
    Ok(())
}

fn trajectory_request(
    origin: &str,
    destination: &str,
    vessel_id: &StableId,
    minimum_days: f64,
    maximum_days: f64,
    departure_samples: u32,
    duration_samples: u32,
) -> Result<TransferRequest> {
    let departure = TdbInstant::from_utc(CalendarDateTime::new(2160, 1, 1, 0, 0, 0, 0)?)?;
    Ok(TransferRequest {
        origin_id: StableId::new(origin)?,
        destination_id: StableId::new(destination)?,
        departure_window: TimeWindow {
            earliest: departure,
            latest: departure.checked_add_micros(30 * MICROS_PER_DAY)?,
        },
        duration_window: DurationWindow {
            minimum_s: minimum_days * 86_400.0,
            maximum_s: maximum_days * 86_400.0,
        },
        vessel_id: vessel_id.clone(),
        payload_mass_kg: MassKilograms::new(100.0)?,
        payload_volume_m3: VolumeCubicMeters::new(1.0)?,
        reserve_policy: ReservePolicy::zero(),
        arrival_condition: ArrivalCondition::Flyby,
        options: SolverOptions {
            departure_samples,
            duration_samples,
            maximum_evaluations: departure_samples.saturating_mul(duration_samples),
            ..SolverOptions::default()
        },
    })
}

fn engineering_audit() -> Result<()> {
    let catalog = EngineeringCatalog::bundled().context("load bundled engineering catalog")?;
    let audit = catalog
        .audit()
        .context("audit bundled engineering catalog")?;
    println!("{}", serde_json::to_string_pretty(&audit)?);
    println!("blueprints:");
    for blueprint in catalog.blueprints() {
        println!(
            "  {:<30} rev={} role={:<24} dry_mass_kg={:<10.0} basis={:?}",
            blueprint.id,
            blueprint.revision,
            blueprint.role,
            blueprint.dry_mass_kg.value(),
            blueprint.engineering_basis,
        );
    }
    println!("engineering catalog audit: PASS");
    Ok(())
}

fn catalog_audit() -> Result<()> {
    let catalog = Catalog::bundled().context("load bundled catalog")?;
    let audit = catalog.audit().context("audit bundled catalog")?;
    println!("{}", serde_json::to_string_pretty(&audit)?);
    println!("bodies:");
    for body in catalog.bodies() {
        println!(
            "  {:<12} {:<20} parent={:<10} class={:?} status={:?}",
            body.id,
            body.canonical_name,
            body.parent_id.as_ref().map_or("—", |id| id.as_str()),
            body.body_class,
            body.development_status
        );
    }
    println!("catalog audit: PASS");
    Ok(())
}

fn ephemeris_verify(span_years: u32) -> Result<()> {
    if span_years == 0 || span_years > 1_000 {
        bail!("span-years must be in 1..=1000");
    }
    let catalog = Arc::new(Catalog::bundled()?);
    let ephemeris = EphemerisService::new(Arc::clone(&catalog));
    let start = TdbInstant::from_utc(CalendarDateTime::new(2060, 1, 1, 0, 0, 0, 0)?)?;
    let span_days = i64::from(span_years) * 365 + i64::from(span_years) / 4;
    let sample_step_days = 25;
    let mut sample_count = 0_u64;
    let mut max_radius_error = 0.0_f64;
    for day in (0..=span_days).step_by(sample_step_days as usize) {
        let time = start.checked_add_micros(day * MICROS_PER_DAY)?;
        let next = time.checked_add_micros(3_600 * 1_000_000)?;
        for body in catalog.bodies() {
            let state = ephemeris.local_state(&body.id, time)?;
            let next_state = ephemeris.local_state(&body.id, next)?;
            if state
                .state
                .position_m
                .iter()
                .chain(state.state.velocity_mps.iter())
                .any(|value| !value.is_finite())
            {
                bail!("non-finite state for {} at day {}", body.id, day);
            }
            if let Some(elements) = &body.ephemeris {
                let radius = norm(state.state.position_m);
                let min_radius = elements.semi_major_axis_m.value() * (1.0 - elements.eccentricity);
                let max_radius = elements.semi_major_axis_m.value() * (1.0 + elements.eccentricity);
                let tolerance = elements.semi_major_axis_m.value() * 1e-9;
                if radius < min_radius - tolerance || radius > max_radius + tolerance {
                    bail!("orbit radius outside conic bounds for {}", body.id);
                }
                let displacement = norm(subtract(
                    next_state.state.position_m,
                    state.state.position_m,
                ));
                let speed_bound =
                    norm(state.state.velocity_mps).max(norm(next_state.state.velocity_mps));
                if displacement > speed_bound * 3_600.0 * 1.01 + 1.0 {
                    bail!("trajectory discontinuity for {} at day {}", body.id, day);
                }
                let nearest_bound = (radius - min_radius).min(max_radius - radius).abs();
                max_radius_error =
                    max_radius_error.max(nearest_bound / elements.semi_major_axis_m.value());
            } else if norm(state.state.position_m) != 0.0 {
                bail!("root body {} is not at the heliocentric origin", body.id);
            }
            sample_count += 1;
        }
    }
    println!("span_years={span_years}");
    println!("bodies={}", catalog.bodies().len());
    println!("samples={sample_count}");
    println!("continuity_step_seconds=3600");
    println!("quality_thresholds=reference:conic-1e-9,approximate:conic-1e-9");
    println!("max_normalized_orbit_span={max_radius_error:.9}");
    println!("ephemeris verification: PASS");
    Ok(())
}

fn replay_empty_world(rates: &[u32]) -> Result<()> {
    if rates.is_empty() {
        bail!("at least one playback rate is required");
    }
    let target = TdbInstant::from_utc(CalendarDateTime::new(2170, 1, 1, 0, 0, 0, 0)?)?;
    let mut expected: Option<String> = None;
    for rate in rates {
        let mut app = SimulationApp::new_standard_2160()?;
        app.set_time_rate(*rate)?;
        app.advance_until(target)?;
        let hash = app.snapshot().deterministic_hash()?;
        if expected.as_ref().is_some_and(|value| value != &hash) {
            bail!("determinism failure at rate {rate}: {hash}");
        }
        expected.get_or_insert_with(|| hash.clone());
        println!("rate={rate:<5} final_hash={hash}");
    }
    println!("empty-world replay: PASS");
    Ok(())
}

fn replay_voyage(rates: &[u32]) -> Result<()> {
    if rates.is_empty() {
        bail!("at least one playback rate is required");
    }
    let mut expected: Option<String> = None;
    for rate in rates {
        let mut app = SimulationApp::new_standard_2160()?;
        let vessel_id = app.primary_vessel()?.id.clone();
        let mut request = trajectory_request("earth", "moon", &vessel_id, 3.0, 40.0, 2, 5)?;
        request.arrival_condition = ArrivalCondition::Rendezvous;
        let report = app.quote_transfer(&request, &CancellationToken::default())?;
        let solution = report
            .solutions
            .first()
            .ok_or_else(|| anyhow::anyhow!("voyage replay has no executable solution"))?
            .clone();
        let arrival = solution.arrival;
        let receipt = app.schedule_voyage(ScheduleVoyageCommand {
            command_id: StableId::new("command:replay-voyage")?,
            expected_world_revision: app.world_revision(),
            request,
            solution,
        })?;
        app.set_time_rate(*rate)?;
        app.advance_until(arrival)?;
        let plan = app
            .voyage_plans()
            .get(&receipt.object_id)
            .ok_or_else(|| anyhow::anyhow!("scheduled voyage disappeared"))?;
        if plan.status != VoyageStatus::Arrived || !app.execution_diagnostics().is_empty() {
            bail!("voyage did not arrive inside tolerance at rate {rate}");
        }
        let hash = app.snapshot().deterministic_hash()?;
        if expected.as_ref().is_some_and(|value| value != &hash) {
            bail!("voyage determinism failure at rate {rate}: {hash}");
        }
        expected.get_or_insert_with(|| hash.clone());
        println!(
            "rate={rate:<5} propellant_kg={:.6} lifetime_s={:.6} final_hash={hash}",
            plan.actual_propellant_consumed_kg,
            plan.actual_reactor_lifetime_used_s + plan.actual_engine_lifetime_used_s,
        );
    }
    println!("voyage replay: PASS");
    Ok(())
}

fn norm(vector: [f64; 3]) -> f64 {
    vector.iter().map(|value| value * value).sum::<f64>().sqrt()
}

fn subtract(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}
