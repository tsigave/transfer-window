use crate::{TransferSolution, ValidationLevel};
use serde::{Deserialize, Serialize};
use sim_time::StableId;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ParetoObjectives {
    pub arrival_tdb_micros: i64,
    pub propellant_kg: f64,
    pub payload_kg: f64,
    pub lifetime_used_s: f64,
    pub estimated_cost_credits: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepresentativeSolutions {
    pub fastest: StableId,
    pub balanced: StableId,
    pub efficient: StableId,
}

pub fn pareto_front(solutions: &[TransferSolution]) -> Vec<&TransferSolution> {
    solutions
        .iter()
        .filter(|solution| solution.validation_level == ValidationLevel::Executable)
        .filter(|candidate| {
            !solutions.iter().any(|other| {
                other.validation_level == ValidationLevel::Executable
                    && other.id != candidate.id
                    && dominates(other.pareto_objectives(), candidate.pareto_objectives())
            })
        })
        .collect()
}

pub fn select_representatives(frontier: &[&TransferSolution]) -> Option<RepresentativeSolutions> {
    if frontier.is_empty() {
        return None;
    }
    let fastest = frontier
        .iter()
        .min_by_key(|solution| solution.arrival.micros_since_j2000())?;
    let efficient = frontier.iter().min_by(|left, right| {
        left.estimated_cost_credits
            .total_cmp(&right.estimated_cost_credits)
            .then_with(|| {
                left.propellant_consumed_kg
                    .total_cmp(&right.propellant_consumed_kg)
            })
    })?;

    let ranges = ObjectiveRanges::from_frontier(frontier);
    let balanced = frontier.iter().min_by(|left, right| {
        balanced_score(left.pareto_objectives(), ranges)
            .total_cmp(&balanced_score(right.pareto_objectives(), ranges))
    })?;
    Some(RepresentativeSolutions {
        fastest: fastest.id.clone(),
        balanced: balanced.id.clone(),
        efficient: efficient.id.clone(),
    })
}

fn dominates(left: ParetoObjectives, right: ParetoObjectives) -> bool {
    let no_worse = left.arrival_tdb_micros <= right.arrival_tdb_micros
        && left.propellant_kg <= right.propellant_kg
        && left.payload_kg >= right.payload_kg
        && left.lifetime_used_s <= right.lifetime_used_s
        && left.estimated_cost_credits <= right.estimated_cost_credits;
    let strictly_better = left.arrival_tdb_micros < right.arrival_tdb_micros
        || left.propellant_kg < right.propellant_kg
        || left.payload_kg > right.payload_kg
        || left.lifetime_used_s < right.lifetime_used_s
        || left.estimated_cost_credits < right.estimated_cost_credits;
    no_worse && strictly_better
}

#[derive(Debug, Clone, Copy)]
struct ObjectiveRanges {
    arrival_min: f64,
    arrival_span: f64,
    propellant_min: f64,
    propellant_span: f64,
    payload_max: f64,
    payload_span: f64,
    lifetime_min: f64,
    lifetime_span: f64,
    cost_min: f64,
    cost_span: f64,
}

impl ObjectiveRanges {
    fn from_frontier(frontier: &[&TransferSolution]) -> Self {
        let objectives: Vec<_> = frontier
            .iter()
            .map(|solution| solution.pareto_objectives())
            .collect();
        let bounds = |values: Vec<f64>| {
            let minimum = values.iter().copied().fold(f64::INFINITY, f64::min);
            let maximum = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            (minimum, (maximum - minimum).max(1.0))
        };
        let (arrival_min, arrival_span) = bounds(
            objectives
                .iter()
                .map(|value| value.arrival_tdb_micros as f64)
                .collect(),
        );
        let (propellant_min, propellant_span) =
            bounds(objectives.iter().map(|value| value.propellant_kg).collect());
        let (payload_min, payload_span) =
            bounds(objectives.iter().map(|value| value.payload_kg).collect());
        let payload_max = payload_min + payload_span;
        let (lifetime_min, lifetime_span) = bounds(
            objectives
                .iter()
                .map(|value| value.lifetime_used_s)
                .collect(),
        );
        let (cost_min, cost_span) = bounds(
            objectives
                .iter()
                .map(|value| value.estimated_cost_credits)
                .collect(),
        );
        Self {
            arrival_min,
            arrival_span,
            propellant_min,
            propellant_span,
            payload_max,
            payload_span,
            lifetime_min,
            lifetime_span,
            cost_min,
            cost_span,
        }
    }
}

fn balanced_score(objectives: ParetoObjectives, ranges: ObjectiveRanges) -> f64 {
    let normalized = [
        (objectives.arrival_tdb_micros as f64 - ranges.arrival_min) / ranges.arrival_span,
        (objectives.propellant_kg - ranges.propellant_min) / ranges.propellant_span,
        (ranges.payload_max - objectives.payload_kg) / ranges.payload_span,
        (objectives.lifetime_used_s - ranges.lifetime_min) / ranges.lifetime_span,
        (objectives.estimated_cost_credits - ranges.cost_min) / ranges.cost_span,
    ];
    normalized
        .iter()
        .map(|value| value * value)
        .sum::<f64>()
        .sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        standard_test_vessel, ArrivalCondition, CancellationToken, DurationWindow, SolverOptions,
        TimeWindow, TrajectorySolver, TransferRequest,
    };
    use sim_engineering::{MassKilograms, ReservePolicy, VolumeCubicMeters};
    use sim_time::{CalendarDateTime, TdbInstant, MICROS_PER_DAY};

    #[test]
    fn representatives_are_members_of_the_real_executable_frontier() {
        let solver = TrajectorySolver::bundled().unwrap();
        let (blueprint, vessel) = standard_test_vessel("ship:lunar-courier").unwrap();
        let departure =
            TdbInstant::from_utc(CalendarDateTime::new(2160, 1, 1, 0, 0, 0, 0).unwrap()).unwrap();
        let request = TransferRequest {
            origin_id: "earth".parse().unwrap(),
            destination_id: "moon".parse().unwrap(),
            departure_window: TimeWindow {
                earliest: departure,
                latest: departure.checked_add_micros(20 * MICROS_PER_DAY).unwrap(),
            },
            duration_window: DurationWindow {
                minimum_s: 3.0 * 86_400.0,
                maximum_s: 40.0 * 86_400.0,
            },
            vessel_id: vessel.id.clone(),
            payload_mass_kg: MassKilograms::new(1_000.0).unwrap(),
            payload_volume_m3: VolumeCubicMeters::new(10.0).unwrap(),
            reserve_policy: ReservePolicy::zero(),
            arrival_condition: ArrivalCondition::Rendezvous,
            options: SolverOptions {
                departure_samples: 3,
                duration_samples: 5,
                maximum_evaluations: 15,
                ..SolverOptions::default()
            },
        };
        let report = solver
            .search(&request, &blueprint, &vessel, &CancellationToken::default())
            .unwrap();
        let frontier = pareto_front(&report.solutions);
        let representatives = select_representatives(&frontier).unwrap();
        let ids: Vec<_> = frontier.iter().map(|solution| &solution.id).collect();
        assert!(ids.contains(&&representatives.fastest));
        assert!(ids.contains(&&representatives.balanced));
        assert!(ids.contains(&&representatives.efficient));
        assert!(frontier
            .iter()
            .all(|solution| solution.validation_level == ValidationLevel::Executable));
    }
}
