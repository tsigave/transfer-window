use serde::{Deserialize, Serialize};
use sim_app::SimulationApp;
use sim_engineering::{MassKilograms, ReservePolicy, VolumeCubicMeters};
use sim_time::{StableId, TdbInstant, MICROS_PER_DAY};
use sim_trajectory::{
    pareto_front, select_representatives, standard_test_vessel, ArrivalCondition,
    CancellationToken, DurationWindow, SolverOptions, TimeWindow, TrajectorySolver,
    TransferRequest,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

const SAVE_FILE_NAME: &str = "alpha-v0.1.transfer-window";
const LEGACY_SAVE_FILE_NAME: &str = "alpha-v0.1.solarstorm";
const LEGACY_APP_IDENTIFIER: &str = "game.solarstorm.alpha";

struct AppState(Mutex<SimulationApp>);
struct PlannerState(Arc<Mutex<BTreeMap<String, CancellationToken>>>);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlanTransferArgs {
    request_id: String,
    origin_id: String,
    destination_id: String,
    departure_tdb_micros: i64,
    payload_mass_kg: f64,
    payload_volume_m3: f64,
    minimum_duration_days: f64,
    maximum_duration_days: f64,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PlannerProgressEvent {
    request_id: String,
    evaluated: u32,
    planned: u32,
    executable_solutions: usize,
    status: sim_trajectory::SearchStatus,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanTransferResult {
    report: sim_trajectory::TransferSearchReport,
    pareto_solution_ids: Vec<StableId>,
    representatives: Option<sim_trajectory::RepresentativeSolutions>,
}

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

#[tauri::command]
async fn plan_transfer(
    app_handle: AppHandle,
    state: State<'_, PlannerState>,
    args: PlanTransferArgs,
) -> Result<PlanTransferResult, String> {
    let cancellation = CancellationToken::default();
    state
        .0
        .lock()
        .map_err(|_| "PLANNER_STATE_POISONED".to_string())?
        .insert(args.request_id.clone(), cancellation.clone());
    let requests = Arc::clone(&state.0);
    tauri::async_runtime::spawn_blocking(move || {
        let result = (|| {
            let solver = TrajectorySolver::bundled().map_err(|error| error.to_string())?;
            let (blueprint, vessel) = standard_test_vessel("ship:lunar-courier")
                .map_err(|error| error.to_string())?;
            let departure = TdbInstant::from_micros_since_j2000(args.departure_tdb_micros);
            let request = TransferRequest {
                origin_id: StableId::new(args.origin_id.clone())
                    .map_err(|error| error.to_string())?,
                destination_id: StableId::new(args.destination_id.clone())
                    .map_err(|error| error.to_string())?,
                departure_window: TimeWindow {
                    earliest: departure,
                    latest: departure
                        .checked_add_micros(30 * MICROS_PER_DAY)
                        .map_err(|error| error.to_string())?,
                },
                duration_window: DurationWindow {
                    minimum_s: args.minimum_duration_days * 86_400.0,
                    maximum_s: args.maximum_duration_days * 86_400.0,
                },
                vessel_id: vessel.id.clone(),
                payload_mass_kg: MassKilograms::new(args.payload_mass_kg)
                    .map_err(|error| error.to_string())?,
                payload_volume_m3: VolumeCubicMeters::new(args.payload_volume_m3)
                    .map_err(|error| error.to_string())?,
                reserve_policy: ReservePolicy::zero(),
                arrival_condition: ArrivalCondition::Rendezvous,
                options: SolverOptions {
                    departure_samples: 3,
                    duration_samples: 5,
                    maximum_evaluations: 15,
                    ..SolverOptions::default()
                },
            };
            let event_request_id = args.request_id.clone();
            let report = solver
                .search_with_progress(
                    &request,
                    &blueprint,
                    &vessel,
                    &cancellation,
                    |progress| {
                        let _ = app_handle.emit(
                            "trajectory-progress",
                            PlannerProgressEvent {
                                request_id: event_request_id.clone(),
                                evaluated: progress.evaluated,
                                planned: progress.planned,
                                executable_solutions: progress.executable_solutions,
                                status: progress.status,
                            },
                        );
                    },
                )
                .map_err(|error| error.to_string())?;
            let frontier = pareto_front(&report.solutions);
            let representatives = select_representatives(&frontier);
            Ok(PlanTransferResult {
                pareto_solution_ids: frontier
                    .iter()
                    .map(|solution| solution.id.clone())
                    .collect(),
                report,
                representatives,
            })
        })();
        if let Ok(mut active) = requests.lock() {
            active.remove(&args.request_id);
        }
        result
    })
    .await
    .map_err(|error| format!("PLANNER_TASK_FAILED: {error}"))?
}

#[tauri::command]
fn cancel_transfer(
    state: State<'_, PlannerState>,
    request_id: String,
) -> Result<bool, String> {
    let active = state
        .0
        .lock()
        .map_err(|_| "PLANNER_STATE_POISONED".to_string())?;
    if let Some(cancellation) = active.get(&request_id) {
        cancellation.cancel();
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let simulation = SimulationApp::new_standard_2160().expect("bundled catalog must be valid");
    tauri::Builder::default()
        .manage(AppState(Mutex::new(simulation)))
        .manage(PlannerState(Arc::new(Mutex::new(BTreeMap::new()))))
        .invoke_handler(tauri::generate_handler![
            list_bodies,
            body_state,
            map_sample,
            save_game,
            load_game,
            plan_transfer,
            cancel_transfer
        ])
        .run(tauri::generate_context!())
        .expect("error while running Transfer Window");
}
