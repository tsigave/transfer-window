//! Application use cases: the only mutation boundary for the authoritative world.

use serde::{Deserialize, Serialize};
use sim_astro::{AstroError, BodyState, Catalog, CelestialBody, EphemerisService};
use sim_time::{
    CalendarDateTime, EventQueue, ScheduledEvent, StableId, TdbInstant, TimeError,
    MICROS_PER_SECOND,
};
use std::collections::BTreeMap;
use std::sync::Arc;

pub const SAVE_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    InvalidId,
    InvalidTime,
    BodyNotFound,
    StaleState,
    PermissionDenied,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameSnapshot {
    pub schema_version: u32,
    pub content_version: String,
    pub simulation_time: TdbInstant,
    pub rng_states: BTreeMap<String, u64>,
    pub world: WorldState,
    pub events: Vec<ScheduledEvent>,
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
    world: WorldState,
    queue: EventQueue,
    rng_states: BTreeMap<String, u64>,
    time_rate: u32,
}

impl SimulationApp {
    pub fn new_standard_2160() -> Result<Self, AppError> {
        let catalog = Arc::new(Catalog::bundled()?);
        let simulation_time = TdbInstant::from_utc(CalendarDateTime::new(2160, 1, 1, 0, 0, 0, 0)?)?;
        let selected_body_id = StableId::new("earth")?;
        let world = WorldState {
            simulation_time,
            selected_body_id,
            revision: 0,
            processed_event_ids: Vec::new(),
        };
        let rng_states = ["economy", "incidents", "research", "ai"]
            .into_iter()
            .enumerate()
            .map(|(index, name)| (name.to_string(), 0x5eed_2160_u64 + index as u64))
            .collect();
        Ok(Self {
            ephemeris: EphemerisService::new(Arc::clone(&catalog)),
            catalog,
            world,
            queue: EventQueue::default(),
            rng_states,
            time_rate: 0,
        })
    }

    pub fn from_snapshot(snapshot: GameSnapshot) -> Result<Self, AppError> {
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
        }
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
            self.world.processed_event_ids.push(event.id);
            self.world.revision += 1;
        }
        self.world.simulation_time = target;
        self.world.revision += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
