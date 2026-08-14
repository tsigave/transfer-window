//! Application use cases: the only mutation boundary for the authoritative world.

use serde::{Deserialize, Serialize};
use sim_astro::{AstroError, BodyState, Catalog, CelestialBody, EphemerisService};
use sim_engineering::{
    DurationSeconds, EnergyJoules, EngineeringCatalog, MassKilograms, ShipBlueprint, VesselState,
    VolumeCubicMeters,
};
use sim_time::{
    CalendarDateTime, EventQueue, ScheduledEvent, StableId, TdbInstant, TimeError,
    MICROS_PER_SECOND,
};
use sim_trajectory::{
    transfer_input_hash, CancellationToken, SearchProgress, TrajectorySegment, TrajectorySolver,
    TransferRequest, TransferSearchReport, TransferSolution, ValidationLevel, SOLVER_VERSION,
};
use std::collections::BTreeMap;
use std::sync::Arc;

pub const SAVE_SCHEMA: u32 = 2;
pub const PREVIOUS_SAVE_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    InvalidId,
    InvalidTime,
    BodyNotFound,
    StaleState,
    PermissionDenied,
    SolutionInvalidated,
    PlanNotFound,
    PlanNotCancellable,
    VesselUnavailable,
    SaveCorrupt,
    SaveUnsupported,
    IoError,
}

#[derive(Debug, thiserror::Error)]
#[error("{code:?}: {message}")]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
    pub field_path: Option<String>,
}

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            field_path: None,
        }
    }

    pub fn at(mut self, path: impl Into<String>) -> Self {
        self.field_path = Some(path.into());
        self
    }
}

impl From<AstroError> for AppError {
    fn from(error: AstroError) -> Self {
        let code = if matches!(error, AstroError::BodyNotFound(_)) {
            ErrorCode::BodyNotFound
        } else {
            ErrorCode::InvalidId
        };
        Self::new(code, error.to_string())
    }
}

impl From<TimeError> for AppError {
    fn from(error: TimeError) -> Self {
        Self::new(ErrorCode::InvalidTime, error.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogFields {
    pub event: String,
    pub simulation_time_tdb_micros: i64,
    pub world_revision: u64,
    pub command_id: Option<StableId>,
    pub object_id: Option<StableId>,
    pub error_code: Option<ErrorCode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldState {
    pub simulation_time: TdbInstant,
    pub selected_body_id: StableId,
    pub revision: u64,
    pub processed_event_ids: Vec<StableId>,
    #[serde(default)]
    pub vessels: BTreeMap<StableId, VesselState>,
    #[serde(default)]
    pub voyage_plans: BTreeMap<StableId, VoyagePlan>,
    #[serde(default)]
    pub execution_diagnostics: Vec<ExecutionDiagnostic>,
    #[serde(default)]
    pub command_receipts: BTreeMap<StableId, CommandReceipt>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameSnapshot {
    pub schema_version: u32,
    pub content_version: String,
    pub simulation_time: TdbInstant,
    pub rng_states: BTreeMap<String, u64>,
    pub world: WorldState,
    pub events: Vec<ScheduledEvent>,
    #[serde(default = "default_solver_version")]
    pub solver_version: String,
}

fn default_solver_version() -> String {
    SOLVER_VERSION.into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoyageStatus {
    Scheduled,
    InProgress,
    Arrived,
    Cancelled,
    Diagnostic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoyagePlan {
    pub id: StableId,
    pub vessel_id: StableId,
    pub solution: TransferSolution,
    pub status: VoyageStatus,
    pub created_at: TdbInstant,
    pub actual_propellant_consumed_kg: f64,
    pub actual_fusion_fuel_consumed_kg: f64,
    pub actual_reactor_lifetime_used_s: f64,
    pub actual_engine_lifetime_used_s: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionDiagnostic {
    pub plan_id: StableId,
    pub at: TdbInstant,
    pub position_error_m: f64,
    pub velocity_error_mps: f64,
    pub propellant_error_kg: f64,
    pub lifetime_error_s: f64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandReceipt {
    pub command_id: StableId,
    pub object_id: StableId,
    pub world_revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleVoyageCommand {
    pub command_id: StableId,
    pub expected_world_revision: u64,
    pub request: TransferRequest,
    pub solution: TransferSolution,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelPlanCommand {
    pub command_id: StableId,
    pub expected_world_revision: u64,
    pub plan_id: StableId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum VoyageEventStage {
    Thrust,
    Coast,
    Approach,
    Arrival,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VoyageEventPayload {
    plan_id: StableId,
    stage: VoyageEventStage,
    propellant_fraction: f64,
    fusion_fuel_fraction: f64,
    reactor_lifetime_fraction: f64,
    engine_lifetime_fraction: f64,
}

impl GameSnapshot {
    pub fn deterministic_hash(&self) -> Result<String, AppError> {
        let bytes = serde_json::to_vec(self)
            .map_err(|error| AppError::new(ErrorCode::IoError, error.to_string()))?;
        Ok(blake3::hash(&bytes).to_hex().to_string())
    }
}

pub struct SimulationApp {
    catalog: Arc<Catalog>,
    ephemeris: EphemerisService,
    engineering: EngineeringCatalog,
    world: WorldState,
    queue: EventQueue,
    rng_states: BTreeMap<String, u64>,
    time_rate: u32,
}

impl SimulationApp {
    pub fn new_standard_2160() -> Result<Self, AppError> {
        let catalog = Arc::new(Catalog::bundled()?);
        let engineering = EngineeringCatalog::bundled()
            .map_err(|error| AppError::new(ErrorCode::InvalidId, error.to_string()))?;
        let simulation_time = TdbInstant::from_utc(CalendarDateTime::new(2160, 1, 1, 0, 0, 0, 0)?)?;
        let selected_body_id = StableId::new("earth")?;
        let ephemeris = EphemerisService::new(Arc::clone(&catalog));
        let vessel = standard_vessel(&engineering, &ephemeris, simulation_time)?;
        let world = WorldState {
            simulation_time,
            selected_body_id,
            revision: 0,
            processed_event_ids: Vec::new(),
            vessels: [(vessel.id.clone(), vessel)].into_iter().collect(),
            voyage_plans: BTreeMap::new(),
            execution_diagnostics: Vec::new(),
            command_receipts: BTreeMap::new(),
        };
        let rng_states = ["economy", "incidents", "research", "ai"]
            .into_iter()
            .enumerate()
            .map(|(index, name)| (name.to_string(), 0x5eed_2160_u64 + index as u64))
            .collect();
        Ok(Self {
            ephemeris,
            catalog,
            engineering,
            world,
            queue: EventQueue::default(),
            rng_states,
            time_rate: 0,
        })
    }

    pub fn from_snapshot(snapshot: GameSnapshot) -> Result<Self, AppError> {
        let snapshot = migrate_snapshot(snapshot)?;
        if snapshot.schema_version != SAVE_SCHEMA {
            return Err(AppError::new(
                ErrorCode::SaveUnsupported,
                format!("save schema {} is not supported", snapshot.schema_version),
            ));
        }
        if snapshot.simulation_time != snapshot.world.simulation_time {
            return Err(AppError::new(
                ErrorCode::SaveCorrupt,
                "snapshot and world times differ",
            ));
        }
        let catalog = Arc::new(Catalog::bundled()?);
        let engineering = EngineeringCatalog::bundled()
            .map_err(|error| AppError::new(ErrorCode::InvalidId, error.to_string()))?;
        if snapshot.content_version != catalog.content_version() {
            return Err(AppError::new(
                ErrorCode::SaveUnsupported,
                format!(
                    "content version {} is not installed",
                    snapshot.content_version
                ),
            ));
        }
        catalog.body(&snapshot.world.selected_body_id)?;
        Ok(Self {
            ephemeris: EphemerisService::new(Arc::clone(&catalog)),
            catalog,
            engineering,
            world: snapshot.world,
            queue: snapshot.events.into(),
            rng_states: snapshot.rng_states,
            time_rate: 0,
        })
    }

    pub fn snapshot(&self) -> GameSnapshot {
        GameSnapshot {
            schema_version: SAVE_SCHEMA,
            content_version: self.catalog.content_version().to_string(),
            simulation_time: self.world.simulation_time,
            rng_states: self.rng_states.clone(),
            world: self.world.clone(),
            events: self.queue.ordered(),
            solver_version: SOLVER_VERSION.into(),
        }
    }

    pub fn world_revision(&self) -> u64 {
        self.world.revision
    }

    pub fn vessels(&self) -> &BTreeMap<StableId, VesselState> {
        &self.world.vessels
    }

    pub fn voyage_plans(&self) -> &BTreeMap<StableId, VoyagePlan> {
        &self.world.voyage_plans
    }

    pub fn execution_diagnostics(&self) -> &[ExecutionDiagnostic] {
        &self.world.execution_diagnostics
    }

    pub fn primary_vessel(&self) -> Result<&VesselState, AppError> {
        self.world
            .vessels
            .values()
            .next()
            .ok_or_else(|| AppError::new(ErrorCode::VesselUnavailable, "world has no vessel"))
    }

    pub fn blueprint_for_vessel(&self, vessel: &VesselState) -> Result<&ShipBlueprint, AppError> {
        self.engineering
            .blueprint(&vessel.blueprint_id, vessel.blueprint_revision)
            .ok_or_else(|| {
                AppError::new(
                    ErrorCode::VesselUnavailable,
                    "vessel blueprint revision is not installed",
                )
            })
    }

    pub fn quote_transfer(
        &self,
        request: &TransferRequest,
        cancellation: &CancellationToken,
    ) -> Result<TransferSearchReport, AppError> {
        self.quote_transfer_with_progress(request, cancellation, |_| {})
    }

    pub fn quote_transfer_with_progress<F>(
        &self,
        request: &TransferRequest,
        cancellation: &CancellationToken,
        progress: F,
    ) -> Result<TransferSearchReport, AppError>
    where
        F: FnMut(SearchProgress),
    {
        let vessel = self.world.vessels.get(&request.vessel_id).ok_or_else(|| {
            AppError::new(
                ErrorCode::VesselUnavailable,
                "request vessel does not exist",
            )
        })?;
        let blueprint = self.blueprint_for_vessel(vessel)?;
        let solver = TrajectorySolver::new(Arc::clone(&self.catalog));
        solver
            .search_with_progress(request, blueprint, vessel, cancellation, progress)
            .map_err(|error| AppError::new(ErrorCode::SolutionInvalidated, error.to_string()))
    }

    pub fn validate_transfer(
        &self,
        request: &TransferRequest,
        solution: &TransferSolution,
    ) -> Result<(), AppError> {
        if solution.validation_level != ValidationLevel::Executable {
            return Err(AppError::new(
                ErrorCode::SolutionInvalidated,
                "only executable solutions may be scheduled",
            ));
        }
        if solution.origin_id != request.origin_id
            || solution.destination_id != request.destination_id
            || solution.payload_mass_kg != request.payload_mass_kg.value()
        {
            return Err(AppError::new(
                ErrorCode::SolutionInvalidated,
                "solution endpoints or payload no longer match the request",
            ));
        }
        if solution.departure < self.world.simulation_time {
            return Err(AppError::new(
                ErrorCode::SolutionInvalidated,
                "solution departure is in the past",
            ));
        }
        let vessel = self.world.vessels.get(&request.vessel_id).ok_or_else(|| {
            AppError::new(
                ErrorCode::SolutionInvalidated,
                "solution vessel no longer exists",
            )
        })?;
        if vessel.active_plan_id.is_some() {
            return Err(AppError::new(
                ErrorCode::VesselUnavailable,
                "vessel already has an active voyage plan",
            ));
        }
        let blueprint = self.blueprint_for_vessel(vessel)?;
        let current_hash = transfer_input_hash(request, blueprint, vessel)
            .map_err(|error| AppError::new(ErrorCode::SolutionInvalidated, error.to_string()))?;
        if current_hash != solution.metadata.input_hash {
            return Err(AppError::new(
                ErrorCode::SolutionInvalidated,
                "SOLUTION_INVALIDATED: vessel, payload, departure, or solver inputs changed",
            ));
        }
        if solution.metadata.solver_version != SOLVER_VERSION {
            return Err(AppError::new(
                ErrorCode::SolutionInvalidated,
                "SOLUTION_INVALIDATED: solver version changed",
            ));
        }
        if solution.metadata.position_tolerance_m
            != request.options.verification_position_tolerance_m
            || solution.metadata.velocity_tolerance_mps
                != request.options.verification_velocity_tolerance_mps
            || solution.margins.position_error_m > solution.metadata.position_tolerance_m
            || solution.margins.velocity_error_mps > solution.metadata.velocity_tolerance_mps
        {
            return Err(AppError::new(
                ErrorCode::SolutionInvalidated,
                "SOLUTION_INVALIDATED: verification tolerances no longer match",
            ));
        }
        Ok(())
    }

    pub fn schedule_voyage(
        &mut self,
        command: ScheduleVoyageCommand,
    ) -> Result<CommandReceipt, AppError> {
        if let Some(receipt) = self.world.command_receipts.get(&command.command_id) {
            return Ok(receipt.clone());
        }
        if command.expected_world_revision != self.world.revision {
            return Err(AppError::new(
                ErrorCode::StaleState,
                "expected world revision does not match",
            ));
        }
        self.validate_transfer(&command.request, &command.solution)?;
        let digest =
            blake3::hash(format!("{}:{}", command.command_id, command.solution.id).as_bytes())
                .to_hex()
                .to_string();
        let plan_id = StableId::new(format!("plan:{}", &digest[..24]))?;
        let events = voyage_events(&plan_id, &command.solution)?;
        let payload_mass_kg = command.request.payload_mass_kg;
        let payload_volume_m3 = command.request.payload_volume_m3;
        let plan = VoyagePlan {
            id: plan_id.clone(),
            vessel_id: command.request.vessel_id.clone(),
            solution: command.solution,
            status: VoyageStatus::Scheduled,
            created_at: self.world.simulation_time,
            actual_propellant_consumed_kg: 0.0,
            actual_fusion_fuel_consumed_kg: 0.0,
            actual_reactor_lifetime_used_s: 0.0,
            actual_engine_lifetime_used_s: 0.0,
        };
        let vessel = self
            .world
            .vessels
            .get_mut(&plan.vessel_id)
            .expect("validated vessel remains present");
        vessel.payload_mass_kg = payload_mass_kg;
        vessel.payload_volume_m3 = payload_volume_m3;
        vessel.active_plan_id = Some(plan_id.clone());
        for event in events {
            self.queue.push(event);
        }
        self.world.voyage_plans.insert(plan_id.clone(), plan);
        self.world.revision += 1;
        let receipt = CommandReceipt {
            command_id: command.command_id.clone(),
            object_id: plan_id,
            world_revision: self.world.revision,
        };
        self.world
            .command_receipts
            .insert(command.command_id, receipt.clone());
        Ok(receipt)
    }

    pub fn cancel_plan(&mut self, command: CancelPlanCommand) -> Result<CommandReceipt, AppError> {
        if let Some(receipt) = self.world.command_receipts.get(&command.command_id) {
            return Ok(receipt.clone());
        }
        if command.expected_world_revision != self.world.revision {
            return Err(AppError::new(
                ErrorCode::StaleState,
                "expected world revision does not match",
            ));
        }
        let plan = self
            .world
            .voyage_plans
            .get_mut(&command.plan_id)
            .ok_or_else(|| AppError::new(ErrorCode::PlanNotFound, "voyage plan does not exist"))?;
        if plan.status != VoyageStatus::Scheduled {
            return Err(AppError::new(
                ErrorCode::PlanNotCancellable,
                "only a scheduled voyage can be cancelled",
            ));
        }
        plan.status = VoyageStatus::Cancelled;
        let vessel_id = plan.vessel_id.clone();
        self.world
            .vessels
            .get_mut(&vessel_id)
            .expect("plan vessel remains present")
            .active_plan_id = None;
        let plan_id = command.plan_id.clone();
        self.queue.retain(|event| {
            serde_json::from_value::<VoyageEventPayload>(event.payload.clone())
                .map_or(true, |payload| payload.plan_id != plan_id)
        });
        self.world.revision += 1;
        let receipt = CommandReceipt {
            command_id: command.command_id.clone(),
            object_id: command.plan_id,
            world_revision: self.world.revision,
        };
        self.world
            .command_receipts
            .insert(command.command_id, receipt.clone());
        Ok(receipt)
    }

    pub fn list_bodies(&self) -> &[CelestialBody] {
        self.catalog.bodies()
    }

    pub fn search_bodies(&self, query: &str) -> Vec<&CelestialBody> {
        self.catalog.search(query)
    }

    pub fn body_state(&self, id: &StableId, time: TdbInstant) -> Result<BodyState, AppError> {
        self.ephemeris.state(id, time).map_err(Into::into)
    }

    pub fn map_sample(&self, time: TdbInstant) -> Result<Vec<BodyState>, AppError> {
        self.ephemeris.map_sample(time).map_err(Into::into)
    }

    pub fn hierarchy(&self, parent: Option<&StableId>) -> Vec<&CelestialBody> {
        self.catalog.children_of(parent)
    }

    pub fn selected_body(&self) -> &StableId {
        &self.world.selected_body_id
    }

    pub fn select_body(&mut self, id: StableId) -> Result<(), AppError> {
        self.catalog.body(&id)?;
        self.world.selected_body_id = id;
        self.world.revision += 1;
        Ok(())
    }

    pub fn simulation_time(&self) -> TdbInstant {
        self.world.simulation_time
    }

    pub fn time_rate(&self) -> u32 {
        self.time_rate
    }

    pub fn set_time_rate(&mut self, rate: u32) -> Result<(), AppError> {
        if ![0, 1, 100, 10_000].contains(&rate) {
            return Err(AppError::new(
                ErrorCode::InvalidTime,
                "time rate must be one of 0, 1, 100, or 10000",
            ));
        }
        self.time_rate = rate;
        Ok(())
    }

    pub fn advance_wall_seconds(&mut self, wall_seconds: u64) -> Result<(), AppError> {
        let simulated_micros = i64::try_from(wall_seconds)
            .ok()
            .and_then(|seconds| seconds.checked_mul(i64::from(self.time_rate)))
            .and_then(|seconds| seconds.checked_mul(MICROS_PER_SECOND))
            .ok_or_else(|| AppError::new(ErrorCode::InvalidTime, "advance would overflow"))?;
        let target = self
            .world
            .simulation_time
            .checked_add_micros(simulated_micros)?;
        self.advance_until(target)
    }

    pub fn schedule_event(&mut self, event: ScheduledEvent) -> Result<(), AppError> {
        if event.due_time < self.world.simulation_time {
            return Err(AppError::new(
                ErrorCode::InvalidTime,
                "cannot schedule an event in the past",
            ));
        }
        self.queue.push(event);
        self.world.revision += 1;
        Ok(())
    }

    /// Advances in event-sized segments. Playback rate never changes event order or simulation steps.
    pub fn advance_until(&mut self, target: TdbInstant) -> Result<(), AppError> {
        if target < self.world.simulation_time {
            return Err(AppError::new(
                ErrorCode::InvalidTime,
                "cannot advance backwards",
            ));
        }
        while let Some(event) = self.queue.pop_due(target) {
            self.world.simulation_time = event.due_time;
            if event.kind.as_str().starts_with("voyage:") {
                self.process_voyage_event(&event)?;
            }
            self.world.processed_event_ids.push(event.id);
            self.world.revision += 1;
        }
        self.world.simulation_time = target;
        self.world.revision += 1;
        Ok(())
    }

    fn process_voyage_event(&mut self, event: &ScheduledEvent) -> Result<(), AppError> {
        let payload: VoyageEventPayload =
            serde_json::from_value(event.payload.clone()).map_err(|error| {
                AppError::new(ErrorCode::SaveCorrupt, format!("voyage event: {error}"))
            })?;
        let plan_snapshot = self
            .world
            .voyage_plans
            .get(&payload.plan_id)
            .cloned()
            .ok_or_else(|| AppError::new(ErrorCode::PlanNotFound, "event plan is missing"))?;
        if matches!(
            plan_snapshot.status,
            VoyageStatus::Cancelled | VoyageStatus::Arrived | VoyageStatus::Diagnostic
        ) {
            return Ok(());
        }
        match payload.stage {
            VoyageEventStage::Thrust => {
                let propellant =
                    plan_snapshot.solution.propellant_consumed_kg * payload.propellant_fraction;
                let fusion =
                    plan_snapshot.solution.fusion_fuel_consumed_kg * payload.fusion_fuel_fraction;
                let reactor_life = plan_snapshot.solution.reactor_lifetime_used_s
                    * payload.reactor_lifetime_fraction;
                let engine_life = plan_snapshot.solution.engine_lifetime_used_s
                    * payload.engine_lifetime_fraction;
                let vessel = self
                    .world
                    .vessels
                    .get_mut(&plan_snapshot.vessel_id)
                    .ok_or_else(|| {
                        AppError::new(ErrorCode::VesselUnavailable, "plan vessel is missing")
                    })?;
                vessel.propellant_kg = MassKilograms::new(
                    vessel.propellant_kg.value() - propellant,
                )
                .map_err(|error| {
                    AppError::new(
                        ErrorCode::SaveCorrupt,
                        format!("execution propellant: {error}"),
                    )
                })?;
                vessel.fusion_fuel_kg = MassKilograms::new(vessel.fusion_fuel_kg.value() - fusion)
                    .map_err(|error| {
                        AppError::new(
                            ErrorCode::SaveCorrupt,
                            format!("execution fusion fuel: {error}"),
                        )
                    })?;
                vessel.reactor_full_power_used_s =
                    DurationSeconds::new(vessel.reactor_full_power_used_s.value() + reactor_life)
                        .map_err(|error| AppError::new(ErrorCode::SaveCorrupt, error.to_string()))?;
                vessel.engine_full_power_used_s =
                    DurationSeconds::new(vessel.engine_full_power_used_s.value() + engine_life)
                        .map_err(|error| {
                            AppError::new(ErrorCode::SaveCorrupt, error.to_string())
                        })?;
                let plan = self
                    .world
                    .voyage_plans
                    .get_mut(&payload.plan_id)
                    .expect("plan remains present");
                plan.status = VoyageStatus::InProgress;
                plan.actual_propellant_consumed_kg += propellant;
                plan.actual_fusion_fuel_consumed_kg += fusion;
                plan.actual_reactor_lifetime_used_s += reactor_life;
                plan.actual_engine_lifetime_used_s += engine_life;
            }
            VoyageEventStage::Coast => {
                let vessel = self
                    .world
                    .vessels
                    .get_mut(&plan_snapshot.vessel_id)
                    .ok_or_else(|| {
                        AppError::new(ErrorCode::VesselUnavailable, "plan vessel is missing")
                    })?;
                vessel.current_continuous_burn_s = DurationSeconds::new(0.0)
                    .map_err(|error| AppError::new(ErrorCode::SaveCorrupt, error.to_string()))?;
                vessel.thermal_buffer_j = EnergyJoules::new(0.0)
                    .map_err(|error| AppError::new(ErrorCode::SaveCorrupt, error.to_string()))?;
                self.world
                    .voyage_plans
                    .get_mut(&payload.plan_id)
                    .expect("plan remains present")
                    .status = VoyageStatus::InProgress;
            }
            VoyageEventStage::Approach => {
                self.world
                    .voyage_plans
                    .get_mut(&payload.plan_id)
                    .expect("plan remains present")
                    .status = VoyageStatus::InProgress;
            }
            VoyageEventStage::Arrival => {
                self.finish_voyage(&payload.plan_id, event.due_time)?;
            }
        }
        Ok(())
    }

    fn finish_voyage(&mut self, plan_id: &StableId, at: TdbInstant) -> Result<(), AppError> {
        let plan_snapshot = self
            .world
            .voyage_plans
            .get(plan_id)
            .cloned()
            .ok_or_else(|| AppError::new(ErrorCode::PlanNotFound, "arrival plan is missing"))?;
        let destination = self
            .ephemeris
            .state(&plan_snapshot.solution.destination_id, at)?;
        let propellant_error = (plan_snapshot.actual_propellant_consumed_kg
            - plan_snapshot.solution.propellant_consumed_kg)
            .abs();
        let lifetime_error = ((plan_snapshot.actual_reactor_lifetime_used_s
            + plan_snapshot.actual_engine_lifetime_used_s)
            - (plan_snapshot.solution.reactor_lifetime_used_s
                + plan_snapshot.solution.engine_lifetime_used_s))
            .abs();
        let position_error = plan_snapshot.solution.margins.position_error_m;
        let velocity_error = plan_snapshot.solution.margins.velocity_error_mps;
        let consumption_tolerance = plan_snapshot.solution.propellant_consumed_kg.max(1.0) * 1e-9;
        let lifetime_tolerance = (plan_snapshot.solution.reactor_lifetime_used_s
            + plan_snapshot.solution.engine_lifetime_used_s)
            .max(1.0)
            * 1e-9;
        let has_diagnostic = position_error > plan_snapshot.solution.metadata.position_tolerance_m
            || velocity_error > plan_snapshot.solution.metadata.velocity_tolerance_mps
            || propellant_error > consumption_tolerance
            || lifetime_error > lifetime_tolerance;
        let vessel = self
            .world
            .vessels
            .get_mut(&plan_snapshot.vessel_id)
            .ok_or_else(|| AppError::new(ErrorCode::VesselUnavailable, "plan vessel is missing"))?;
        let mut actual_state = destination.state;
        actual_state.position_m[0] += position_error;
        actual_state.velocity_mps[0] += velocity_error;
        vessel.state_vector = Some(actual_state);
        vessel.active_plan_id = None;
        let plan = self
            .world
            .voyage_plans
            .get_mut(plan_id)
            .expect("plan remains present");
        plan.status = if has_diagnostic {
            VoyageStatus::Diagnostic
        } else {
            VoyageStatus::Arrived
        };
        if has_diagnostic {
            self.world.execution_diagnostics.push(ExecutionDiagnostic {
                plan_id: plan_id.clone(),
                at,
                position_error_m: position_error,
                velocity_error_mps: velocity_error,
                propellant_error_kg: propellant_error,
                lifetime_error_s: lifetime_error,
                message: "execution ended outside the approved tolerance; state was not snapped"
                    .into(),
            });
        }
        Ok(())
    }
}

pub fn migrate_snapshot(mut snapshot: GameSnapshot) -> Result<GameSnapshot, AppError> {
    if snapshot.schema_version == SAVE_SCHEMA {
        return Ok(snapshot);
    }
    if snapshot.schema_version != PREVIOUS_SAVE_SCHEMA {
        return Err(AppError::new(
            ErrorCode::SaveUnsupported,
            format!("save schema {} is not supported", snapshot.schema_version),
        ));
    }
    let catalog = Arc::new(Catalog::bundled()?);
    if snapshot.content_version != catalog.content_version() {
        return Err(AppError::new(
            ErrorCode::SaveUnsupported,
            "legacy save content version is not installed",
        ));
    }
    let engineering = EngineeringCatalog::bundled()
        .map_err(|error| AppError::new(ErrorCode::SaveCorrupt, error.to_string()))?;
    let ephemeris = EphemerisService::new(catalog);
    if snapshot.world.vessels.is_empty() {
        let vessel = standard_vessel(&engineering, &ephemeris, snapshot.simulation_time)?;
        snapshot.world.vessels.insert(vessel.id.clone(), vessel);
    }
    snapshot.schema_version = SAVE_SCHEMA;
    snapshot.solver_version = SOLVER_VERSION.into();
    Ok(snapshot)
}

fn standard_vessel(
    engineering: &EngineeringCatalog,
    ephemeris: &EphemerisService,
    at: TdbInstant,
) -> Result<VesselState, AppError> {
    let blueprint_id = StableId::new("ship:lunar-courier")?;
    let blueprint = engineering.blueprint(&blueprint_id, 1).ok_or_else(|| {
        AppError::new(
            ErrorCode::SaveCorrupt,
            "standard Lunar Courier blueprint is missing",
        )
    })?;
    let earth = ephemeris.state(&StableId::new("earth")?, at)?;
    Ok(VesselState {
        id: StableId::new("vessel:player-lunar-courier")?,
        blueprint_id: blueprint.id.clone(),
        blueprint_revision: blueprint.revision,
        state_vector: Some(earth.state),
        payload_mass_kg: MassKilograms::new(0.0)
            .map_err(|error| AppError::new(ErrorCode::SaveCorrupt, error.to_string()))?,
        payload_volume_m3: VolumeCubicMeters::new(0.0)
            .map_err(|error| AppError::new(ErrorCode::SaveCorrupt, error.to_string()))?,
        fusion_fuel_kg: blueprint.fusion_fuel_capacity_kg,
        propellant_kg: blueprint.propellant_capacity_kg,
        thermal_buffer_j: EnergyJoules::new(0.0)
            .map_err(|error| AppError::new(ErrorCode::SaveCorrupt, error.to_string()))?,
        current_continuous_burn_s: DurationSeconds::new(0.0)
            .map_err(|error| AppError::new(ErrorCode::SaveCorrupt, error.to_string()))?,
        reactor_full_power_used_s: DurationSeconds::new(0.0)
            .map_err(|error| AppError::new(ErrorCode::SaveCorrupt, error.to_string()))?,
        engine_full_power_used_s: DurationSeconds::new(0.0)
            .map_err(|error| AppError::new(ErrorCode::SaveCorrupt, error.to_string()))?,
        active_plan_id: None,
    })
}

fn voyage_events(
    plan_id: &StableId,
    solution: &TransferSolution,
) -> Result<Vec<ScheduledEvent>, AppError> {
    let total_powered_duration = solution
        .segments
        .iter()
        .filter_map(|segment| match segment {
            TrajectorySegment::FiniteBurn {
                powered_duration_s, ..
            } => Some(*powered_duration_s),
            _ => None,
        })
        .sum::<f64>();
    let digest = blake3::hash(plan_id.as_str().as_bytes())
        .to_hex()
        .to_string();
    let mut events = Vec::new();
    for segment in &solution.segments {
        let (due_time, priority, kind, payload) = match segment {
            TrajectorySegment::FiniteBurn {
                end,
                powered_duration_s,
                ..
            } => {
                let fraction = if total_powered_duration > 0.0 {
                    *powered_duration_s / total_powered_duration
                } else {
                    0.0
                };
                (
                    *end,
                    10,
                    "voyage:thrust",
                    VoyageEventPayload {
                        plan_id: plan_id.clone(),
                        stage: VoyageEventStage::Thrust,
                        propellant_fraction: fraction,
                        fusion_fuel_fraction: fraction,
                        reactor_lifetime_fraction: fraction,
                        engine_lifetime_fraction: fraction,
                    },
                )
            }
            TrajectorySegment::Coast { end, .. } => (
                *end,
                20,
                "voyage:coast",
                VoyageEventPayload {
                    plan_id: plan_id.clone(),
                    stage: VoyageEventStage::Coast,
                    propellant_fraction: 0.0,
                    fusion_fuel_fraction: 0.0,
                    reactor_lifetime_fraction: 0.0,
                    engine_lifetime_fraction: 0.0,
                },
            ),
            TrajectorySegment::Approach { at, .. } => (
                *at,
                30,
                "voyage:approach",
                VoyageEventPayload {
                    plan_id: plan_id.clone(),
                    stage: VoyageEventStage::Approach,
                    propellant_fraction: 0.0,
                    fusion_fuel_fraction: 0.0,
                    reactor_lifetime_fraction: 0.0,
                    engine_lifetime_fraction: 0.0,
                },
            ),
        };
        let index = events.len();
        events.push(ScheduledEvent {
            id: StableId::new(format!("event:{}:{index}", &digest[..20]))?,
            due_time,
            priority,
            kind: StableId::new(kind)?,
            payload_version: 1,
            payload: serde_json::to_value(payload)
                .map_err(|error| AppError::new(ErrorCode::IoError, error.to_string()))?,
        });
    }
    let index = events.len();
    events.push(ScheduledEvent {
        id: StableId::new(format!("event:{}:{index}", &digest[..20]))?,
        due_time: solution.arrival,
        priority: 40,
        kind: StableId::new("voyage:arrival")?,
        payload_version: 1,
        payload: serde_json::to_value(VoyageEventPayload {
            plan_id: plan_id.clone(),
            stage: VoyageEventStage::Arrival,
            propellant_fraction: 0.0,
            fusion_fuel_fraction: 0.0,
            reactor_lifetime_fraction: 0.0,
            engine_lifetime_fraction: 0.0,
        })
        .map_err(|error| AppError::new(ErrorCode::IoError, error.to_string()))?,
    });
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_engineering::{MassKilograms, ReservePolicy, VolumeCubicMeters};
    use sim_time::MICROS_PER_DAY;
    use sim_trajectory::{ArrivalCondition, DurationWindow, SolverOptions, TimeWindow};

    fn transfer_request(app: &SimulationApp) -> TransferRequest {
        let departure = app
            .simulation_time()
            .checked_add_micros(MICROS_PER_DAY)
            .unwrap();
        TransferRequest {
            origin_id: StableId::new("earth").unwrap(),
            destination_id: StableId::new("moon").unwrap(),
            departure_window: TimeWindow {
                earliest: departure,
                latest: departure.checked_add_micros(10 * MICROS_PER_DAY).unwrap(),
            },
            duration_window: DurationWindow {
                minimum_s: 3.0 * 86_400.0,
                maximum_s: 40.0 * 86_400.0,
            },
            vessel_id: app.primary_vessel().unwrap().id.clone(),
            payload_mass_kg: MassKilograms::new(1_000.0).unwrap(),
            payload_volume_m3: VolumeCubicMeters::new(10.0).unwrap(),
            reserve_policy: ReservePolicy::zero(),
            arrival_condition: ArrivalCondition::Rendezvous,
            options: SolverOptions {
                departure_samples: 2,
                duration_samples: 5,
                maximum_evaluations: 10,
                ..SolverOptions::default()
            },
        }
    }

    fn quoted_solution(app: &SimulationApp, request: &TransferRequest) -> TransferSolution {
        let report = app
            .quote_transfer(request, &CancellationToken::default())
            .unwrap();
        report
            .solutions
            .first()
            .unwrap_or_else(|| panic!("no executable solution: {:#?}", report.failures))
            .clone()
    }

    #[test]
    fn standard_world_has_every_catalog_body_and_search_targets() {
        let app = SimulationApp::new_standard_2160().unwrap();
        assert_eq!(app.list_bodies().len(), 41);
        for query in ["地球", "谷神星", "木卫四", "海卫一", "阿罗科特"] {
            assert_eq!(app.search_bodies(query).len(), 1, "{query}");
        }
    }

    #[test]
    fn playback_rates_produce_identical_empty_world_hashes() {
        let target =
            TdbInstant::from_utc(CalendarDateTime::new(2170, 1, 1, 0, 0, 0, 0).unwrap()).unwrap();
        let mut hashes = Vec::new();
        for rate in [0, 1, 100, 10_000] {
            let mut app = SimulationApp::new_standard_2160().unwrap();
            app.set_time_rate(rate).unwrap();
            app.advance_until(target).unwrap();
            hashes.push(app.snapshot().deterministic_hash().unwrap());
        }
        assert!(hashes.windows(2).all(|pair| pair[0] == pair[1]));
    }

    #[test]
    fn scheduled_events_are_replayed_deterministically() {
        let mut app = SimulationApp::new_standard_2160().unwrap();
        let due = app.simulation_time().checked_add_micros(10).unwrap();
        for (id, priority) in [("event:b", 5), ("event:a", 5), ("event:first", 0)] {
            app.schedule_event(ScheduledEvent {
                id: StableId::new(id).unwrap(),
                due_time: due,
                priority,
                kind: StableId::new("test").unwrap(),
                payload_version: 1,
                payload: serde_json::Value::Null,
            })
            .unwrap();
        }
        app.advance_until(due).unwrap();
        let ids: Vec<_> = app
            .snapshot()
            .world
            .processed_event_ids
            .into_iter()
            .map(|id| id.to_string())
            .collect();
        assert_eq!(ids, ["event:first", "event:a", "event:b"]);
    }

    #[test]
    fn stale_solution_fails_without_world_or_resource_mutation() {
        let mut app = SimulationApp::new_standard_2160().unwrap();
        let request = transfer_request(&app);
        let solution = quoted_solution(&app, &request);
        let before = app.snapshot().deterministic_hash().unwrap();
        let mut changed_request = request;
        changed_request.payload_mass_kg = MassKilograms::new(2_000.0).unwrap();
        let error = app
            .schedule_voyage(ScheduleVoyageCommand {
                command_id: StableId::new("command:invalidated").unwrap(),
                expected_world_revision: app.world_revision(),
                request: changed_request,
                solution,
            })
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::SolutionInvalidated);
        assert_eq!(app.snapshot().deterministic_hash().unwrap(), before);
    }

    #[test]
    fn schedule_and_cancel_are_idempotent_and_remove_future_events() {
        let mut app = SimulationApp::new_standard_2160().unwrap();
        let request = transfer_request(&app);
        let solution = quoted_solution(&app, &request);
        let command = ScheduleVoyageCommand {
            command_id: StableId::new("command:schedule").unwrap(),
            expected_world_revision: app.world_revision(),
            request,
            solution,
        };
        let receipt = app.schedule_voyage(command.clone()).unwrap();
        assert_eq!(
            app.primary_vessel().unwrap().payload_mass_kg.value(),
            1_000.0
        );
        assert_eq!(
            app.primary_vessel().unwrap().payload_volume_m3.value(),
            10.0
        );
        assert_eq!(
            app.schedule_voyage(command).unwrap().object_id,
            receipt.object_id
        );
        assert!(!app.snapshot().events.is_empty());
        let cancel = CancelPlanCommand {
            command_id: StableId::new("command:cancel").unwrap(),
            expected_world_revision: app.world_revision(),
            plan_id: receipt.object_id.clone(),
        };
        let cancel_receipt = app.cancel_plan(cancel.clone()).unwrap();
        assert_eq!(
            app.cancel_plan(cancel).unwrap().object_id,
            cancel_receipt.object_id
        );
        assert!(app.snapshot().events.is_empty());
        assert_eq!(
            app.voyage_plans().get(&receipt.object_id).unwrap().status,
            VoyageStatus::Cancelled
        );
        assert!(app.primary_vessel().unwrap().active_plan_id.is_none());
    }

    #[test]
    fn voyage_arrival_hash_is_identical_across_playback_rates() {
        let mut hashes = Vec::new();
        for rate in [1, 100, 10_000] {
            let mut app = SimulationApp::new_standard_2160().unwrap();
            let request = transfer_request(&app);
            let solution = quoted_solution(&app, &request);
            let arrival = solution.arrival;
            let receipt = app
                .schedule_voyage(ScheduleVoyageCommand {
                    command_id: StableId::new("command:replay-voyage").unwrap(),
                    expected_world_revision: app.world_revision(),
                    request,
                    solution,
                })
                .unwrap();
            app.set_time_rate(rate).unwrap();
            app.advance_until(arrival).unwrap();
            let plan = app.voyage_plans().get(&receipt.object_id).unwrap();
            assert_eq!(plan.status, VoyageStatus::Arrived);
            assert!(app.execution_diagnostics().is_empty());
            hashes.push(app.snapshot().deterministic_hash().unwrap());
        }
        assert!(hashes.windows(2).all(|pair| pair[0] == pair[1]));
    }

    #[test]
    fn out_of_tolerance_arrival_keeps_actual_state_and_creates_diagnostic() {
        let mut app = SimulationApp::new_standard_2160().unwrap();
        let request = transfer_request(&app);
        let solution = quoted_solution(&app, &request);
        let arrival = solution.arrival;
        let destination = app
            .body_state(&StableId::new("moon").unwrap(), arrival)
            .unwrap();
        let receipt = app
            .schedule_voyage(ScheduleVoyageCommand {
                command_id: StableId::new("command:diagnostic-voyage").unwrap(),
                expected_world_revision: app.world_revision(),
                request,
                solution,
            })
            .unwrap();
        app.world
            .voyage_plans
            .get_mut(&receipt.object_id)
            .unwrap()
            .solution
            .metadata
            .position_tolerance_m = 0.1;
        app.advance_until(arrival).unwrap();

        assert_eq!(
            app.voyage_plans().get(&receipt.object_id).unwrap().status,
            VoyageStatus::Diagnostic
        );
        assert_eq!(app.execution_diagnostics().len(), 1);
        assert_ne!(
            app.primary_vessel()
                .unwrap()
                .state_vector
                .as_ref()
                .unwrap()
                .position_m,
            destination.state.position_m
        );
    }
}
