//! SQLite snapshots written through a same-directory temporary file and atomic replacement.

use rusqlite::{params, Connection, OptionalExtension};
use sim_app::{migrate_snapshot, GameSnapshot, PREVIOUS_SAVE_SCHEMA, SAVE_SCHEMA};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("SAVE_IO: {0}")]
    Io(#[from] std::io::Error),
    #[error("SAVE_DATABASE: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("SAVE_CORRUPT: {0}")]
    Corrupt(String),
    #[error("SAVE_UNSUPPORTED: schema {0} is not supported")]
    Unsupported(u32),
    #[error("SAVE_INTERRUPTED: injected interruption before atomic replacement")]
    InjectedInterruption,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultInjection {
    None,
    AfterSnapshotWrite,
}

pub fn save_atomic(path: &Path, snapshot: &GameSnapshot) -> Result<(), SaveError> {
    save_atomic_with_fault(path, snapshot, FaultInjection::None)
}

pub fn save_atomic_with_fault(
    path: &Path,
    snapshot: &GameSnapshot,
    fault: FaultInjection,
) -> Result<(), SaveError> {
    if snapshot.schema_version != SAVE_SCHEMA {
        return Err(SaveError::Unsupported(snapshot.schema_version));
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let temporary = tempfile::Builder::new()
        .prefix(".transfer-window-save-")
        .suffix(".sqlite.tmp")
        .tempfile_in(parent)?;
    let temporary_path = temporary.into_temp_path();
    {
        let mut connection = Connection::open(&temporary_path)?;
        connection.pragma_update(None, "journal_mode", "DELETE")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        create_schema(&connection)?;
        let transaction = connection.transaction()?;
        let payload =
            serde_json::to_vec(snapshot).map_err(|error| SaveError::Corrupt(error.to_string()))?;
        let checksum = blake3::hash(&payload).to_hex().to_string();
        transaction.execute(
            "INSERT INTO game_snapshot (slot, schema_version, content_version, simulation_time_tdb_micros, payload_json, checksum_blake3) VALUES (1, ?1, ?2, ?3, ?4, ?5)",
            params![snapshot.schema_version, snapshot.content_version, snapshot.simulation_time.micros_since_j2000(), payload, checksum],
        )?;
        for (sequence, event) in snapshot.events.iter().enumerate() {
            let event_json =
                serde_json::to_vec(event).map_err(|error| SaveError::Corrupt(error.to_string()))?;
            transaction.execute(
                "INSERT INTO event_queue (sequence, event_id, due_time_tdb_micros, priority, payload_json) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![sequence as i64, event.id.as_str(), event.due_time.micros_since_j2000(), event.priority, event_json],
            )?;
        }
        for (name, state) in &snapshot.rng_states {
            transaction.execute(
                "INSERT INTO rng_stream (name, state_u64) VALUES (?1, ?2)",
                params![name, state.to_string()],
            )?;
        }
        transaction.commit()?;
        connection.execute_batch("PRAGMA wal_checkpoint(FULL);")?;
    }
    if fault == FaultInjection::AfterSnapshotWrite {
        return Err(SaveError::InjectedInterruption);
    }
    temporary_path
        .persist(path)
        .map_err(|error| SaveError::Io(error.error))?;
    sync_parent(parent)?;
    Ok(())
}

pub fn load(path: &Path) -> Result<GameSnapshot, SaveError> {
    if !path.exists() {
        return Err(SaveError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "save does not exist",
        )));
    }
    let connection = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let integrity: String = connection.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        return Err(SaveError::Corrupt(format!(
            "SQLite integrity check: {integrity}"
        )));
    }
    let row: Option<(u32, String, Vec<u8>, String)> = connection
        .query_row(
            "SELECT schema_version, content_version, payload_json, checksum_blake3 FROM game_snapshot WHERE slot = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let (schema_version, content_version, payload, expected_checksum) =
        row.ok_or_else(|| SaveError::Corrupt("missing game_snapshot row".into()))?;
    if schema_version != SAVE_SCHEMA && schema_version != PREVIOUS_SAVE_SCHEMA {
        return Err(SaveError::Unsupported(schema_version));
    }
    let actual_checksum = blake3::hash(&payload).to_hex().to_string();
    if actual_checksum != expected_checksum {
        return Err(SaveError::Corrupt("snapshot checksum mismatch".into()));
    }
    let snapshot: GameSnapshot = serde_json::from_slice(&payload)
        .map_err(|error| SaveError::Corrupt(format!("snapshot JSON: {error}")))?;
    if snapshot.schema_version != schema_version || snapshot.content_version != content_version {
        return Err(SaveError::Corrupt(
            "metadata does not match snapshot payload".into(),
        ));
    }
    let event_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM event_queue", [], |row| row.get(0))?;
    let rng_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM rng_stream", [], |row| row.get(0))?;
    if event_count != snapshot.events.len() as i64 || rng_count != snapshot.rng_states.len() as i64
    {
        return Err(SaveError::Corrupt(
            "normalized queue or RNG state does not match snapshot".into(),
        ));
    }
    drop(connection);
    if schema_version == PREVIOUS_SAVE_SCHEMA {
        let backup = migration_backup_path(path);
        if !backup.exists() {
            std::fs::copy(path, &backup)?;
        }
    }
    migrate_snapshot(snapshot).map_err(|error| SaveError::Corrupt(error.to_string()))
}

pub fn migration_backup_path(path: &Path) -> PathBuf {
    let mut backup = path.as_os_str().to_os_string();
    backup.push(".pre-v0.2.bak");
    PathBuf::from(backup)
}

fn create_schema(connection: &Connection) -> Result<(), rusqlite::Error> {
    connection.execute_batch(
        "
        CREATE TABLE save_metadata (
            key TEXT PRIMARY KEY NOT NULL,
            value TEXT NOT NULL
        ) STRICT;
        INSERT INTO save_metadata (key, value) VALUES ('format', 'transfer-window-sqlite-v2');
        CREATE TABLE game_snapshot (
            slot INTEGER PRIMARY KEY CHECK (slot = 1),
            schema_version INTEGER NOT NULL,
            content_version TEXT NOT NULL,
            simulation_time_tdb_micros INTEGER NOT NULL,
            payload_json BLOB NOT NULL,
            checksum_blake3 TEXT NOT NULL
        ) STRICT;
        CREATE TABLE event_queue (
            sequence INTEGER PRIMARY KEY,
            event_id TEXT UNIQUE NOT NULL,
            due_time_tdb_micros INTEGER NOT NULL,
            priority INTEGER NOT NULL,
            payload_json BLOB NOT NULL
        ) STRICT;
        CREATE TABLE rng_stream (
            name TEXT PRIMARY KEY NOT NULL,
            state_u64 TEXT NOT NULL
        ) STRICT;
        ",
    )
}

#[cfg(unix)]
fn sync_parent(parent: &Path) -> Result<(), std::io::Error> {
    std::fs::File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent(_parent: &Path) -> Result<(), std::io::Error> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_app::{ScheduleVoyageCommand, SimulationApp};
    use sim_engineering::{MassKilograms, ReservePolicy, VolumeCubicMeters};
    use sim_time::{CalendarDateTime, StableId, MICROS_PER_DAY};
    use sim_trajectory::{
        ArrivalCondition, CancellationToken, DurationWindow, SolverOptions, TimeWindow,
        TransferRequest,
    };

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
                latest: departure,
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
                departure_samples: 1,
                duration_samples: 5,
                maximum_evaluations: 5,
                ..SolverOptions::default()
            },
        }
    }

    fn write_legacy_schema_one(path: &Path, snapshot: &GameSnapshot) {
        let mut value = serde_json::to_value(snapshot).unwrap();
        let object = value.as_object_mut().unwrap();
        object.insert("schema_version".into(), serde_json::json!(1));
        object.remove("solver_version");
        let world = object.get_mut("world").unwrap().as_object_mut().unwrap();
        for field in [
            "vessels",
            "voyage_plans",
            "execution_diagnostics",
            "command_receipts",
        ] {
            world.remove(field);
        }
        let payload = serde_json::to_vec(&value).unwrap();
        let checksum = blake3::hash(&payload).to_hex().to_string();
        let mut connection = Connection::open(path).unwrap();
        create_schema(&connection).unwrap();
        let transaction = connection.transaction().unwrap();
        transaction.execute(
            "INSERT INTO game_snapshot (slot, schema_version, content_version, simulation_time_tdb_micros, payload_json, checksum_blake3) VALUES (1, 1, ?1, ?2, ?3, ?4)",
            params![snapshot.content_version, snapshot.simulation_time.micros_since_j2000(), payload, checksum],
        ).unwrap();
        for (sequence, event) in snapshot.events.iter().enumerate() {
            transaction.execute(
                "INSERT INTO event_queue (sequence, event_id, due_time_tdb_micros, priority, payload_json) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![sequence as i64, event.id.as_str(), event.due_time.micros_since_j2000(), event.priority, serde_json::to_vec(event).unwrap()],
            ).unwrap();
        }
        for (name, state) in &snapshot.rng_states {
            transaction
                .execute(
                    "INSERT INTO rng_stream (name, state_u64) VALUES (?1, ?2)",
                    params![name, state.to_string()],
                )
                .unwrap();
        }
        transaction.commit().unwrap();
    }

    #[test]
    fn save_load_round_trip_preserves_time_selection_and_hash() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("campaign.transfer-window");
        let mut app = SimulationApp::new_standard_2160().unwrap();
        app.select_body("callisto".parse().unwrap()).unwrap();
        let target =
            sim_time::TdbInstant::from_utc(CalendarDateTime::new(2170, 1, 1, 0, 0, 0, 0).unwrap())
                .unwrap();
        app.advance_until(target).unwrap();
        let snapshot = app.snapshot();
        save_atomic(&path, &snapshot).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(
            loaded.deterministic_hash().unwrap(),
            snapshot.deterministic_hash().unwrap()
        );
        assert_eq!(loaded.world.selected_body_id.as_str(), "callisto");
    }

    #[test]
    fn interrupted_write_leaves_last_complete_save_loadable() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("campaign.transfer-window");
        let app = SimulationApp::new_standard_2160().unwrap();
        let original = app.snapshot();
        save_atomic(&path, &original).unwrap();
        let mut changed = original.clone();
        changed.world.revision += 99;
        let error = save_atomic_with_fault(&path, &changed, FaultInjection::AfterSnapshotWrite)
            .unwrap_err();
        assert!(matches!(error, SaveError::InjectedInterruption));
        assert_eq!(
            load(&path).unwrap().deterministic_hash().unwrap(),
            original.deterministic_hash().unwrap()
        );
    }

    #[test]
    fn successful_atomic_replacement_updates_existing_save() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("campaign.transfer-window");
        let mut app = SimulationApp::new_standard_2160().unwrap();
        save_atomic(&path, &app.snapshot()).unwrap();
        app.select_body("triton".parse().unwrap()).unwrap();
        let replacement = app.snapshot();
        save_atomic(&path, &replacement).unwrap();
        assert_eq!(
            load(&path).unwrap().deterministic_hash().unwrap(),
            replacement.deterministic_hash().unwrap()
        );
    }

    #[test]
    fn corrupt_save_is_reported_instead_of_reset() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("campaign.transfer-window");
        std::fs::write(&path, b"not sqlite").unwrap();
        assert!(load(&path).is_err());
    }

    #[test]
    fn schema_one_save_is_backed_up_and_migrated_with_a_standard_vessel() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("legacy.transfer-window");
        let snapshot = SimulationApp::new_standard_2160().unwrap().snapshot();
        write_legacy_schema_one(&path, &snapshot);
        let original = std::fs::read(&path).unwrap();

        let migrated = load(&path).unwrap();
        assert_eq!(migrated.schema_version, SAVE_SCHEMA);
        assert_eq!(migrated.world.vessels.len(), 1);
        assert_eq!(
            std::fs::read(migration_backup_path(&path)).unwrap(),
            original
        );
        SimulationApp::from_snapshot(migrated).unwrap();
    }

    #[test]
    fn active_voyage_save_load_reaches_the_same_arrival_hash() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("active-voyage.transfer-window");
        let mut uninterrupted = SimulationApp::new_standard_2160().unwrap();
        let request = transfer_request(&uninterrupted);
        let report = uninterrupted
            .quote_transfer(&request, &CancellationToken::default())
            .unwrap();
        let solution = report.solutions.first().unwrap().clone();
        let arrival = solution.arrival;
        uninterrupted
            .schedule_voyage(ScheduleVoyageCommand {
                command_id: StableId::new("command:save-mid-voyage").unwrap(),
                expected_world_revision: uninterrupted.world_revision(),
                request,
                solution,
            })
            .unwrap();
        let midpoint = uninterrupted
            .simulation_time()
            .checked_add_micros(
                (arrival.micros_since_j2000()
                    - uninterrupted.simulation_time().micros_since_j2000())
                    / 2,
            )
            .unwrap();
        uninterrupted.advance_until(midpoint).unwrap();
        save_atomic(&path, &uninterrupted.snapshot()).unwrap();
        let mut resumed = SimulationApp::from_snapshot(load(&path).unwrap()).unwrap();

        uninterrupted.advance_until(arrival).unwrap();
        resumed.advance_until(arrival).unwrap();
        let resumed_snapshot = resumed.snapshot();
        let uninterrupted_snapshot = uninterrupted.snapshot();
        assert_eq!(
            serde_json::to_value(&resumed_snapshot).unwrap(),
            serde_json::to_value(&uninterrupted_snapshot).unwrap()
        );
        assert_eq!(
            resumed_snapshot.deterministic_hash().unwrap(),
            uninterrupted_snapshot.deterministic_hash().unwrap()
        );
    }
}
