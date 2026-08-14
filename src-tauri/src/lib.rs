use serde::{Deserialize, Serialize};
use sim_app::SimulationApp;
use sim_time::{StableId, TdbInstant};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State};

const SAVE_FILE_NAME: &str = "alpha-v0.1.transfer-window";
const LEGACY_SAVE_FILE_NAME: &str = "alpha-v0.1.solarstorm";
const LEGACY_APP_IDENTIFIER: &str = "game.solarstorm.alpha";

struct AppState(Mutex<SimulationApp>);

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct BrowserViewState {
    schema_version: u32,
    content_version: String,
    epoch_tdb_micros: i64,
    selected_body_id: String,
}

#[tauri::command]
fn list_bodies(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let app = state
        .0
        .lock()
        .map_err(|_| "APP_STATE_POISONED".to_string())?;
    serde_json::to_value(app.list_bodies()).map_err(|error| error.to_string())
}

#[tauri::command]
fn body_state(
    state: State<'_, AppState>,
    body_id: String,
    epoch_tdb_micros: i64,
) -> Result<serde_json::Value, String> {
    let app = state
        .0
        .lock()
        .map_err(|_| "APP_STATE_POISONED".to_string())?;
    let id = StableId::new(body_id).map_err(|error| error.to_string())?;
    let value = app
        .body_state(&id, TdbInstant::from_micros_since_j2000(epoch_tdb_micros))
        .map_err(|error| error.to_string())?;
    serde_json::to_value(value).map_err(|error| error.to_string())
}

#[tauri::command]
fn map_sample(
    state: State<'_, AppState>,
    epoch_tdb_micros: i64,
) -> Result<serde_json::Value, String> {
    let app = state
        .0
        .lock()
        .map_err(|_| "APP_STATE_POISONED".to_string())?;
    let value = app
        .map_sample(TdbInstant::from_micros_since_j2000(epoch_tdb_micros))
        .map_err(|error| error.to_string())?;
    serde_json::to_value(value).map_err(|error| error.to_string())
}

#[tauri::command]
fn save_game(
    app_handle: AppHandle,
    state: State<'_, AppState>,
    view_state: BrowserViewState,
) -> Result<(), String> {
    let mut app = state
        .0
        .lock()
        .map_err(|_| "APP_STATE_POISONED".to_string())?;
    let target = TdbInstant::from_micros_since_j2000(view_state.epoch_tdb_micros);
    if target < app.simulation_time() {
        *app = SimulationApp::new_standard_2160().map_err(|error| error.to_string())?;
    }
    app.advance_until(target)
        .map_err(|error| error.to_string())?;
    app.select_body(StableId::new(view_state.selected_body_id).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    let directory = app_handle
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    let path = directory.join(SAVE_FILE_NAME);
    sim_save::save_atomic(&path, &app.snapshot()).map_err(|error| error.to_string())
}

fn find_save_path(directory: &Path) -> PathBuf {
    let current = directory.join(SAVE_FILE_NAME);
    if current.exists() {
        return current;
    }
    let legacy_in_current_directory = directory.join(LEGACY_SAVE_FILE_NAME);
    if legacy_in_current_directory.exists() {
        return legacy_in_current_directory;
    }
    if let Some(data_root) = directory.parent() {
        let legacy_in_previous_app_directory = data_root
            .join(LEGACY_APP_IDENTIFIER)
            .join(LEGACY_SAVE_FILE_NAME);
        if legacy_in_previous_app_directory.exists() {
            return legacy_in_previous_app_directory;
        }
    }
    current
}

#[tauri::command]
fn load_game(app_handle: AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    let directory = app_handle
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    let current_path = directory.join(SAVE_FILE_NAME);
    let source_path = find_save_path(&directory);
    let snapshot = sim_save::load(&source_path).map_err(|error| error.to_string())?;
    if source_path != current_path {
        sim_save::save_atomic(&current_path, &snapshot).map_err(|error| error.to_string())?;
    }
    let view = BrowserViewState {
        schema_version: snapshot.schema_version,
        content_version: snapshot.content_version.clone(),
        epoch_tdb_micros: snapshot.simulation_time.micros_since_j2000(),
        selected_body_id: snapshot.world.selected_body_id.to_string(),
    };
    *state
        .0
        .lock()
        .map_err(|_| "APP_STATE_POISONED".to_string())? =
        SimulationApp::from_snapshot(snapshot).map_err(|error| error.to_string())?;
    serde_json::to_string(&view).map_err(|error| error.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let simulation = SimulationApp::new_standard_2160().expect("bundled catalog must be valid");
    tauri::Builder::default()
        .manage(AppState(Mutex::new(simulation)))
        .invoke_handler(tauri::generate_handler![
            list_bodies,
            body_state,
            map_sample,
            save_game,
            load_game
        ])
        .run(tauri::generate_context!())
        .expect("error while running Transfer Window");
}
