//! SQLite snapshots written through a same-directory temporary file and atomic replacement.

use rusqlite::{params, Connection, OptionalExtension};
use sim_app::{GameSnapshot, SAVE_SCHEMA};
use std::path::Path;

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
        .prefix(".solarstorm-save-")
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
    if schema_version != SAVE_SCHEMA {
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
    Ok(snapshot)
}

fn create_schema(connection: &Connection) -> Result<(), rusqlite::Error> {
    connection.execute_batch(
        "
        CREATE TABLE save_metadata (
            key TEXT PRIMARY KEY NOT NULL,
            value TEXT NOT NULL
        ) STRICT;
        INSERT INTO save_metadata (key, value) VALUES ('format', 'solarstorm-sqlite-v1');
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
    use sim_app::SimulationApp;
    use sim_time::CalendarDateTime;

    #[test]
    fn save_load_round_trip_preserves_time_selection_and_hash() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("campaign.solarstorm");
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
        let path = directory.path().join("campaign.solarstorm");
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
        let path = directory.path().join("campaign.solarstorm");
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
        let path = directory.path().join("campaign.solarstorm");
        std::fs::write(&path, b"not sqlite").unwrap();
        assert!(load(&path).is_err());
    }
}
