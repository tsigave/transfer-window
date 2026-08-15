//! Versioned HTTP boundary for the authoritative simulation.

use async_stream::stream;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sim_app::{
    AppError, CancelPlanCommand, CommandReceipt, ErrorCode, GameSnapshot, ScheduleVoyageCommand,
    SimulationApp,
};
use sim_engineering::{MassKilograms, ReservePolicy, VolumeCubicMeters};
use sim_time::{StableId, TdbInstant, MICROS_PER_DAY};
use sim_trajectory::{
    pareto_front, select_representatives, ArrivalCondition, CancellationToken, DurationWindow,
    SearchProgress, SearchStatus, SolverOptions, TimeWindow, TrajectorySolver, TransferRequest,
    TransferSearchReport, TransferSolution,
};
use std::collections::BTreeMap;
use std::convert::Infallible;
use std::path::{Path as FilePath, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;

pub const API_VERSION: &str = "v1";
const SAVE_FILE_NAME: &str = "alpha-v0.2.transfer-window";

#[derive(Clone)]
pub struct ServerState {
    simulation: Arc<Mutex<SimulationApp>>,
    jobs: Arc<Mutex<BTreeMap<String, PlannerJob>>>,
    save_path: Arc<PathBuf>,
}

impl ServerState {
    pub fn new(data_directory: impl AsRef<FilePath>) -> Result<Self, ApiError> {
        std::fs::create_dir_all(data_directory.as_ref())
            .map_err(|error| ApiError::internal("DATA_DIRECTORY_FAILED", error.to_string()))?;
        Ok(Self {
            simulation: Arc::new(Mutex::new(SimulationApp::new_standard_2160()?)),
            jobs: Arc::new(Mutex::new(BTreeMap::new())),
            save_path: Arc::new(data_directory.as_ref().join(SAVE_FILE_NAME)),
        })
    }
}

pub fn router(state: ServerState, allowed_origin: &str) -> Result<Router, ApiError> {
    let origin = HeaderValue::from_str(allowed_origin)
        .map_err(|error| ApiError::internal("INVALID_WEB_ORIGIN", error.to_string()))?;
    let cors = CorsLayer::new()
        .allow_origin(origin)
        .allow_headers([header::CONTENT_TYPE])
        .allow_methods([Method::GET, Method::POST, Method::DELETE]);
    Ok(Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/bodies", get(list_bodies))
        .route("/api/v1/bodies/{body_id}/state", get(body_state))
        .route("/api/v1/map-sample", get(map_sample))
        .route("/api/v1/world", get(world_snapshot))
        .route("/api/v1/simulation/advance", post(advance_simulation))
        .route("/api/v1/saves/default", post(save_game).get(load_game))
        .route("/api/v1/trajectory/jobs", post(create_trajectory_job))
        .route(
            "/api/v1/trajectory/jobs/{request_id}",
            get(trajectory_job).delete(cancel_trajectory_job),
        )
        .route(
            "/api/v1/trajectory/jobs/{request_id}/events",
            get(trajectory_events),
        )
        .route("/api/v1/voyages", post(schedule_voyage))
        .route("/api/v1/voyages/{plan_id}/cancel", post(cancel_voyage))
        .layer(cors)
        .with_state(state))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct HealthResponse {
    status: &'static str,
    api_version: &'static str,
    project_version: &'static str,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        api_version: API_VERSION,
        project_version: env!("CARGO_PKG_VERSION"),
    })
}

async fn list_bodies(
    State(state): State<ServerState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let app = lock(&state.simulation, "APP_STATE_POISONED")?;
    Ok(Json(serde_json::to_value(app.list_bodies())?))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EpochQuery {
    epoch_tdb_micros: i64,
}

async fn body_state(
    State(state): State<ServerState>,
    Path(body_id): Path<String>,
    Query(query): Query<EpochQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let app = lock(&state.simulation, "APP_STATE_POISONED")?;
    let body_id = StableId::new(body_id)?;
    Ok(Json(serde_json::to_value(app.body_state(
        &body_id,
        TdbInstant::from_micros_since_j2000(query.epoch_tdb_micros),
    )?)?))
}

async fn map_sample(
    State(state): State<ServerState>,
    Query(query): Query<EpochQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let app = lock(&state.simulation, "APP_STATE_POISONED")?;
    Ok(Json(serde_json::to_value(app.map_sample(
        TdbInstant::from_micros_since_j2000(query.epoch_tdb_micros),
    )?)?))
}

async fn world_snapshot(State(state): State<ServerState>) -> Result<Json<GameSnapshot>, ApiError> {
    Ok(Json(
        lock(&state.simulation, "APP_STATE_POISONED")?.snapshot(),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdvanceSimulationArgs {
    target_tdb_micros: i64,
}

async fn advance_simulation(
    State(state): State<ServerState>,
    Json(args): Json<AdvanceSimulationArgs>,
) -> Result<Json<GameSnapshot>, ApiError> {
    let mut app = lock(&state.simulation, "APP_STATE_POISONED")?;
    app.advance_until(TdbInstant::from_micros_since_j2000(args.target_tdb_micros))?;
    Ok(Json(app.snapshot()))
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebViewState {
    pub schema_version: u32,
    pub content_version: String,
    pub epoch_tdb_micros: i64,
    pub selected_body_id: String,
}

async fn save_game(
    State(state): State<ServerState>,
    Json(view): Json<WebViewState>,
) -> Result<Json<WebViewState>, ApiError> {
    let mut app = lock(&state.simulation, "APP_STATE_POISONED")?;
    let current = app.snapshot();
    if view.schema_version != sim_app::SAVE_SCHEMA {
        return Err(ApiError::bad_request(
            "SAVE_UNSUPPORTED",
            format!("view schema {} is not supported", view.schema_version),
        ));
    }
    if view.content_version != current.content_version {
        return Err(ApiError::bad_request(
            "SAVE_UNSUPPORTED",
            "view content version is not installed",
        ));
    }
    let target = TdbInstant::from_micros_since_j2000(view.epoch_tdb_micros);
    app.advance_until(target)?;
    app.select_body(StableId::new(view.selected_body_id.clone())?)?;
    sim_save::save_atomic(&state.save_path, &app.snapshot())?;
    Ok(Json(view))
}

async fn load_game(State(state): State<ServerState>) -> Result<Json<WebViewState>, ApiError> {
    let snapshot = sim_save::load(&state.save_path)?;
    let view = WebViewState {
        schema_version: snapshot.schema_version,
        content_version: snapshot.content_version.clone(),
        epoch_tdb_micros: snapshot.simulation_time.micros_since_j2000(),
        selected_body_id: snapshot.world.selected_body_id.to_string(),
    };
    *lock(&state.simulation, "APP_STATE_POISONED")? = SimulationApp::from_snapshot(snapshot)?;
    Ok(Json(view))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanTransferArgs {
    pub request_id: String,
    pub origin_id: String,
    pub destination_id: String,
    pub departure_tdb_micros: i64,
    pub payload_mass_kg: f64,
    pub payload_volume_m3: f64,
    pub minimum_duration_days: f64,
    pub maximum_duration_days: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannerProgressEvent {
    pub request_id: String,
    pub evaluated: u32,
    pub planned: u32,
    pub executable_solutions: usize,
    pub status: SearchStatus,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanTransferResult {
    pub report: TransferSearchReport,
    pub pareto_solution_ids: Vec<StableId>,
    pub representatives: Option<sim_trajectory::RepresentativeSolutions>,
    pub request: TransferRequest,
    pub world_revision: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
enum PlannerLifecycle {
    Running,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlannerJobView {
    request_id: String,
    state: PlannerLifecycle,
    progress: PlannerProgressEvent,
    result: Option<PlanTransferResult>,
    error: Option<ApiProblem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
enum PlannerEvent {
    Progress(PlannerProgressEvent),
    Complete,
    Cancelled,
    Failed(ApiProblem),
}

impl PlannerEvent {
    fn event_name(&self) -> &'static str {
        match self {
            Self::Progress(_) => "progress",
            Self::Complete => "complete",
            Self::Cancelled => "cancelled",
            Self::Failed(_) => "failed",
        }
    }
}

struct PlannerJob {
    cancellation: CancellationToken,
    lifecycle: PlannerLifecycle,
    progress: PlannerProgressEvent,
    result: Option<PlanTransferResult>,
    error: Option<ApiProblem>,
    events: broadcast::Sender<PlannerEvent>,
}

impl PlannerJob {
    fn view(&self) -> PlannerJobView {
        PlannerJobView {
            request_id: self.progress.request_id.clone(),
            state: self.lifecycle.clone(),
            progress: self.progress.clone(),
            result: self.result.clone(),
            error: self.error.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CreatedJob {
    request_id: String,
    events_url: String,
}

async fn create_trajectory_job(
    State(state): State<ServerState>,
    Json(args): Json<PlanTransferArgs>,
) -> Result<(StatusCode, Json<CreatedJob>), ApiError> {
    if args.request_id.trim().is_empty() {
        return Err(ApiError::bad_request(
            "INVALID_REQUEST_ID",
            "requestId cannot be empty",
        ));
    }
    StableId::new(args.request_id.clone())?;
    let cancellation = CancellationToken::default();
    let (events, _) = broadcast::channel(32);
    {
        let mut jobs = lock(&state.jobs, "PLANNER_STATE_POISONED")?;
        if jobs.contains_key(&args.request_id) {
            return Err(ApiError::conflict(
                "REQUEST_ID_CONFLICT",
                "requestId already exists",
            ));
        }
        if jobs.len() >= 128 {
            jobs.retain(|_, job| matches!(&job.lifecycle, PlannerLifecycle::Running));
        }
        if jobs.len() >= 128 {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "TOO_MANY_TRAJECTORY_JOBS",
                "128 trajectory jobs are still running",
            ));
        }
        jobs.insert(
            args.request_id.clone(),
            PlannerJob {
                cancellation: cancellation.clone(),
                lifecycle: PlannerLifecycle::Running,
                progress: PlannerProgressEvent {
                    request_id: args.request_id.clone(),
                    evaluated: 0,
                    planned: 15,
                    executable_solutions: 0,
                    status: SearchStatus::Completed,
                },
                result: None,
                error: None,
                events,
            },
        );
    }
    let request_id = args.request_id.clone();
    let worker_state = state.clone();
    tokio::task::spawn_blocking(move || run_trajectory_job(worker_state, args, cancellation));
    Ok((
        StatusCode::ACCEPTED,
        Json(CreatedJob {
            request_id: request_id.clone(),
            events_url: format!("/api/v1/trajectory/jobs/{request_id}/events"),
        }),
    ))
}

fn run_trajectory_job(state: ServerState, args: PlanTransferArgs, cancellation: CancellationToken) {
    let request_id = args.request_id.clone();
    let outcome = (|| -> Result<PlanTransferResult, ApiError> {
        let (blueprint, vessel, world_revision) = {
            let app = lock(&state.simulation, "APP_STATE_POISONED")?;
            let vessel = app.primary_vessel()?.clone();
            let blueprint = app.blueprint_for_vessel(&vessel)?.clone();
            (blueprint, vessel, app.world_revision())
        };
        let departure = TdbInstant::from_micros_since_j2000(args.departure_tdb_micros);
        let request = TransferRequest {
            origin_id: StableId::new(args.origin_id)?,
            destination_id: StableId::new(args.destination_id)?,
            departure_window: TimeWindow {
                earliest: departure,
                latest: departure.checked_add_micros(30 * MICROS_PER_DAY)?,
            },
            duration_window: DurationWindow {
                minimum_s: args.minimum_duration_days * 86_400.0,
                maximum_s: args.maximum_duration_days * 86_400.0,
            },
            vessel_id: vessel.id.clone(),
            payload_mass_kg: MassKilograms::new(args.payload_mass_kg)
                .map_err(|error| ApiError::bad_request("INVALID_PAYLOAD", error.to_string()))?,
            payload_volume_m3: VolumeCubicMeters::new(args.payload_volume_m3)
                .map_err(|error| ApiError::bad_request("INVALID_PAYLOAD", error.to_string()))?,
            reserve_policy: ReservePolicy::zero(),
            arrival_condition: ArrivalCondition::Rendezvous,
            options: SolverOptions {
                departure_samples: 3,
                duration_samples: 5,
                maximum_evaluations: 15,
                ..SolverOptions::default()
            },
        };
        let solver = TrajectorySolver::bundled()
            .map_err(|error| ApiError::bad_request("TRAJECTORY_FAILED", error.to_string()))?;
        let jobs = Arc::clone(&state.jobs);
        let progress_request_id = request_id.clone();
        let report = solver
            .search_with_progress(
                &request,
                &blueprint,
                &vessel,
                &cancellation,
                move |progress| update_progress(&jobs, &progress_request_id, progress),
            )
            .map_err(|error| ApiError::bad_request("TRAJECTORY_FAILED", error.to_string()))?;
        let frontier = pareto_front(&report.solutions);
        Ok(PlanTransferResult {
            pareto_solution_ids: frontier
                .iter()
                .map(|solution| solution.id.clone())
                .collect(),
            representatives: select_representatives(&frontier),
            report,
            request,
            world_revision,
        })
    })();
    if let Ok(mut jobs) = state.jobs.lock() {
        if let Some(job) = jobs.get_mut(&request_id) {
            match outcome {
                Ok(result) => {
                    let cancelled = result.report.status == SearchStatus::Cancelled;
                    job.lifecycle = if cancelled {
                        PlannerLifecycle::Cancelled
                    } else {
                        PlannerLifecycle::Completed
                    };
                    job.result = Some(result);
                    let _ = job.events.send(if cancelled {
                        PlannerEvent::Cancelled
                    } else {
                        PlannerEvent::Complete
                    });
                }
                Err(error) => {
                    let problem = error.problem;
                    job.lifecycle = PlannerLifecycle::Failed;
                    job.error = Some(problem.clone());
                    let _ = job.events.send(PlannerEvent::Failed(problem));
                }
            }
        }
    }
}

fn update_progress(
    jobs: &Arc<Mutex<BTreeMap<String, PlannerJob>>>,
    request_id: &str,
    progress: SearchProgress,
) {
    if let Ok(mut jobs) = jobs.lock() {
        if let Some(job) = jobs.get_mut(request_id) {
            job.progress = PlannerProgressEvent {
                request_id: request_id.into(),
                evaluated: progress.evaluated,
                planned: progress.planned,
                executable_solutions: progress.executable_solutions,
                status: progress.status,
            };
            let _ = job
                .events
                .send(PlannerEvent::Progress(job.progress.clone()));
        }
    }
}

async fn trajectory_job(
    State(state): State<ServerState>,
    Path(request_id): Path<String>,
) -> Result<Json<PlannerJobView>, ApiError> {
    let jobs = lock(&state.jobs, "PLANNER_STATE_POISONED")?;
    let job = jobs
        .get(&request_id)
        .ok_or_else(|| ApiError::not_found("JOB_NOT_FOUND", "trajectory job does not exist"))?;
    Ok(Json(job.view()))
}

async fn cancel_trajectory_job(
    State(state): State<ServerState>,
    Path(request_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let jobs = lock(&state.jobs, "PLANNER_STATE_POISONED")?;
    let job = jobs
        .get(&request_id)
        .ok_or_else(|| ApiError::not_found("JOB_NOT_FOUND", "trajectory job does not exist"))?;
    job.cancellation.cancel();
    Ok(StatusCode::ACCEPTED)
}

async fn trajectory_events(
    State(state): State<ServerState>,
    Path(request_id): Path<String>,
) -> Result<Sse<impl futures_core::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let (initial, terminal, mut receiver) = {
        let jobs = lock(&state.jobs, "PLANNER_STATE_POISONED")?;
        let job = jobs
            .get(&request_id)
            .ok_or_else(|| ApiError::not_found("JOB_NOT_FOUND", "trajectory job does not exist"))?;
        let terminal = match &job.lifecycle {
            PlannerLifecycle::Running => None,
            PlannerLifecycle::Completed => Some(PlannerEvent::Complete),
            PlannerLifecycle::Cancelled => Some(PlannerEvent::Cancelled),
            PlannerLifecycle::Failed => job.error.clone().map(PlannerEvent::Failed),
        };
        (
            PlannerEvent::Progress(job.progress.clone()),
            terminal,
            job.events.subscribe(),
        )
    };
    let events = stream! {
        yield Ok(event_to_sse(&initial));
        if let Some(event) = terminal {
            yield Ok(event_to_sse(&event));
            return;
        }
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    let terminal = !matches!(event, PlannerEvent::Progress(_));
                    yield Ok(event_to_sse(&event));
                    if terminal { break; }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Ok(Sse::new(events).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(10))
            .text("transfer-window"),
    ))
}

fn event_to_sse(event: &PlannerEvent) -> Event {
    Event::default()
        .event(event.event_name())
        .data(serde_json::to_string(event).expect("planner event serializes"))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScheduleVoyageArgs {
    command_id: String,
    expected_world_revision: u64,
    request: TransferRequest,
    solution: TransferSolution,
}

async fn schedule_voyage(
    State(state): State<ServerState>,
    Json(args): Json<ScheduleVoyageArgs>,
) -> Result<Json<CommandReceipt>, ApiError> {
    let receipt =
        lock(&state.simulation, "APP_STATE_POISONED")?.schedule_voyage(ScheduleVoyageCommand {
            command_id: StableId::new(args.command_id)?,
            expected_world_revision: args.expected_world_revision,
            request: args.request,
            solution: args.solution,
        })?;
    Ok(Json(receipt))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CancelVoyageArgs {
    command_id: String,
    expected_world_revision: u64,
}

async fn cancel_voyage(
    State(state): State<ServerState>,
    Path(plan_id): Path<String>,
    Json(args): Json<CancelVoyageArgs>,
) -> Result<Json<CommandReceipt>, ApiError> {
    let receipt =
        lock(&state.simulation, "APP_STATE_POISONED")?.cancel_plan(CancelPlanCommand {
            command_id: StableId::new(args.command_id)?,
            expected_world_revision: args.expected_world_revision,
            plan_id: StableId::new(plan_id)?,
        })?;
    Ok(Json(receipt))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiProblem {
    code: String,
    message: String,
    field_path: Option<String>,
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    problem: ApiProblem,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.problem.code, self.problem.message)
    }
}

impl std::error::Error for ApiError {}

impl ApiError {
    fn new(status: StatusCode, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status,
            problem: ApiProblem {
                code: code.into(),
                message: message.into(),
                field_path: None,
            },
        }
    }

    fn bad_request(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, message)
    }

    fn conflict(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, code, message)
    }

    fn not_found(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, code, message)
    }

    fn internal(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, code, message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.problem)).into_response()
    }
}

impl From<AppError> for ApiError {
    fn from(error: AppError) -> Self {
        let status = match error.code {
            ErrorCode::BodyNotFound | ErrorCode::PlanNotFound => StatusCode::NOT_FOUND,
            ErrorCode::StaleState
            | ErrorCode::SolutionInvalidated
            | ErrorCode::PlanNotCancellable => StatusCode::CONFLICT,
            ErrorCode::PermissionDenied => StatusCode::FORBIDDEN,
            ErrorCode::IoError | ErrorCode::SaveCorrupt => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::BAD_REQUEST,
        };
        let code = serde_json::to_value(error.code)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_else(|| "APP_ERROR".into());
        Self {
            status,
            problem: ApiProblem {
                code,
                message: error.message,
                field_path: error.field_path,
            },
        }
    }
}

impl From<sim_time::TimeError> for ApiError {
    fn from(error: sim_time::TimeError) -> Self {
        Self::bad_request("INVALID_TIME", error.to_string())
    }
}

impl From<sim_save::SaveError> for ApiError {
    fn from(error: sim_save::SaveError) -> Self {
        match error {
            sim_save::SaveError::Io(io) if io.kind() == std::io::ErrorKind::NotFound => {
                Self::not_found("SAVE_NOT_FOUND", io.to_string())
            }
            sim_save::SaveError::Unsupported(schema) => Self::bad_request(
                "SAVE_UNSUPPORTED",
                format!("schema {schema} is not supported"),
            ),
            other => Self::internal("SAVE_FAILED", other.to_string()),
        }
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(error: serde_json::Error) -> Self {
        Self::internal("SERIALIZATION_FAILED", error.to_string())
    }
}

fn lock<'a, T>(
    mutex: &'a Mutex<T>,
    code: &'static str,
) -> Result<std::sync::MutexGuard<'a, T>, ApiError> {
    mutex
        .lock()
        .map_err(|_| ApiError::internal(code, "shared state lock was poisoned"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use serde_json::Value;
    use tower::ServiceExt;

    fn test_router() -> (Router, tempfile::TempDir) {
        let directory = tempfile::tempdir().unwrap();
        let state = ServerState::new(directory.path()).unwrap();
        (router(state, "http://localhost:1420").unwrap(), directory)
    }

    async fn json(response: Response) -> Value {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn health_and_authoritative_body_state_are_available_over_http() {
        let (app, _directory) = test_router();
        let health = app
            .clone()
            .oneshot(Request::get("/api/v1/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(health.status(), StatusCode::OK);
        assert_eq!(json(health).await["apiVersion"], "v1");

        let body = app
            .oneshot(
                Request::get("/api/v1/bodies/earth/state?epochTdbMicros=0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(body.status(), StatusCode::OK);
        assert_eq!(json(body).await["body_id"], "earth");
    }

    #[tokio::test]
    async fn save_and_load_use_the_server_side_sqlite_slot() {
        let (app, _directory) = test_router();
        let payload = serde_json::json!({
            "schemaVersion": 2,
            "contentVersion": "solar-system-2026.08.1",
            "epochTdbMicros": 5049129642184000_i64,
            "selectedBodyId": "moon"
        });
        let saved = app
            .clone()
            .oneshot(
                Request::post("/api/v1/saves/default")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(saved.status(), StatusCode::OK, "{:?}", json(saved).await);
        let loaded = app
            .oneshot(
                Request::get("/api/v1/saves/default")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(loaded.status(), StatusCode::OK);
        assert_eq!(json(loaded).await["selectedBodyId"], "moon");
    }

    #[tokio::test]
    async fn trajectory_job_schedules_and_executes_an_authoritative_voyage() {
        let (app, _directory) = test_router();
        let request_id = "integration-moon";
        let create = serde_json::json!({
            "requestId": request_id,
            "originId": "earth",
            "destinationId": "moon",
            "departureTdbMicros": 5049129642184000_i64,
            "payloadMassKg": 1000,
            "payloadVolumeM3": 10,
            "minimumDurationDays": 3,
            "maximumDurationDays": 40
        });
        let created = app
            .clone()
            .oneshot(
                Request::post("/api/v1/trajectory/jobs")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(create.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::ACCEPTED);

        let mut job = Value::Null;
        for _ in 0..50 {
            let response = app
                .clone()
                .oneshot(
                    Request::get(format!("/api/v1/trajectory/jobs/{request_id}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            job = json(response).await;
            if job["state"] != "running" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(job["state"], "completed", "{job:#}");
        let result = &job["result"];
        assert_eq!(
            result["report"]["solutions"][0]["metadata"]["solver_version"],
            "transfer-window-trajectory-v1"
        );

        let events = app
            .clone()
            .oneshot(
                Request::get(format!("/api/v1/trajectory/jobs/{request_id}/events"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(events.status(), StatusCode::OK);
        let event_body = to_bytes(events.into_body(), usize::MAX).await.unwrap();
        let event_body = String::from_utf8(event_body.to_vec()).unwrap();
        assert!(event_body.contains("event: progress"));
        assert!(event_body.contains("event: complete"));

        let solution = result["report"]["solutions"][0].clone();
        let schedule = serde_json::json!({
            "commandId": "command:http-integration",
            "expectedWorldRevision": result["worldRevision"],
            "request": result["request"],
            "solution": solution.clone()
        });
        let scheduled = app
            .clone()
            .oneshot(
                Request::post("/api/v1/voyages")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(schedule.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            scheduled.status(),
            StatusCode::OK,
            "{:?}",
            json(scheduled).await
        );

        let advance = serde_json::json!({ "targetTdbMicros": solution["arrival"] });
        let advanced = app
            .oneshot(
                Request::post("/api/v1/simulation/advance")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(advance.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(advanced.status(), StatusCode::OK);
        let snapshot = json(advanced).await;
        let plan = snapshot["world"]["voyage_plans"]
            .as_object()
            .unwrap()
            .values()
            .next()
            .unwrap();
        assert_eq!(plan["status"], "arrived");
    }
}
