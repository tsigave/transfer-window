use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use sim_app::SimulationApp;
use sim_astro::{Catalog, EphemerisService};
use sim_engineering::EngineeringCatalog;
use sim_time::{CalendarDateTime, TdbInstant, MICROS_PER_DAY};
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
    }
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

fn norm(vector: [f64; 3]) -> f64 {
    vector.iter().map(|value| value * value).sum::<f64>().sqrt()
}

fn subtract(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}
