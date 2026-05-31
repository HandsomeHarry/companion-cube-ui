use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use ccube_core::agents::{curator, reflector};
use ccube_core::llm::LlmBackend;
use ccube_core::{agents::detector, briefing, db, memory, paths::DataRoot};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tower_http::cors::CorsLayer;
use include_dir::{Dir, include_dir};

/// Frontend build output, embedded into the binary at compile time.
/// Requires `npm run build` to have produced `build/` before `cargo build`.
static UI_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../build");

/// Shared application state for all HTTP handlers.
pub struct AppState {
    pub data_root: DataRoot,
    pub start_time: std::time::Instant,
    pub shutdown_token: CancellationToken,
    pub version: &'static str,
    /// Frozen at startup — "memory never changes mid-session" (spec §15).
    pub frozen_profile: String,
    pub frozen_patterns: String,
    pub frozen_patterns_hash: String,
    /// LLM client for detector calls (10s timeout).
    pub llm: Arc<dyn LlmBackend>,
    /// LLM client for curator calls (120s timeout).
    pub curator_llm: Arc<dyn LlmBackend>,
    /// Signalled by the capture loop when an app-focus event arrives.
    pub detector_trigger: Arc<Notify>,
    /// Serializes curator runs (only one at a time).
    pub curator_mutex: Arc<tokio::sync::Mutex<()>>,
    /// Hour of day (0-23, local time) to run scheduled curator. Default 5 (5 AM).
    pub curator_schedule_hour: u32,
    /// Cached LLM-generated session summaries (auto-refreshed every 5 min).
    pub cached_summaries: Arc<tokio::sync::RwLock<Option<SummariesResponse>>>,
}

/// Build the axum router with all endpoints.
pub fn router(state: Arc<AppState>) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/activity", get(activity))
        .route("/briefing", get(get_briefing))
        .route("/detect", post(detect))
        .route("/memory/profile", get(memory_profile))
        .route("/memory/patterns", get(memory_patterns))
        .route("/memory/patterns/history", get(patterns_history))
        .route("/shutdown", post(shutdown))
        .route("/corrections", get(list_corrections_handler).post(create_correction))
        .route("/corrections/{id}", get(get_correction_handler))
        .route("/corrections/group", post(create_group_correction))
        .route("/decisions", get(list_decisions_handler))
        .route("/agents/curator/run", post(run_curator_handler))
        .route("/agents/reflector/run", post(run_reflector_handler))
        .route("/agents/reflector/pending", get(get_pending_handler))
        .route("/agents/reflector/accept", post(accept_pending_handler))
        .route("/agents/reflector/reject", post(reject_pending_handler))
        .route("/config/llm", get(get_llm_config).put(set_llm_config))
        .route("/summaries", get(get_summaries))
        .route("/summarize", post(run_summarize_handler))
        .route("/rhythm", get(get_rhythm))
        .with_state(state);

    Router::new()
        .nest("/api", api)
        .fallback(serve_frontend)
        .layer(CorsLayer::permissive())
}

/// Serve embedded frontend files. Falls back to index.html for SPA client-side routes.
async fn serve_frontend(req: axum::extract::Request) -> axum::response::Response {
    let raw = req.uri().path();
    let path = if raw == "/" { "index.html" } else { raw.trim_start_matches('/') };

    if let Some(file) = UI_DIR.get_file(path) {
        return file_response(path, file.contents());
    }
    // SPA fallback: unknown non-API route -> index.html
    if let Some(index) = UI_DIR.get_file("index.html") {
        return file_response("index.html", index.contents());
    }
    (axum::http::StatusCode::NOT_FOUND, "UI not embedded").into_response()
}

fn file_response(path: &str, bytes: &'static [u8]) -> axum::response::Response {
    let content_type = match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("webp") => "image/webp",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    };
    (
        [(axum::http::header::CONTENT_TYPE, content_type)],
        bytes,
    )
        .into_response()
}

// ---------- Response types ----------

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    uptime_s: u64,
    daemon_version: &'static str,
}

#[derive(Deserialize)]
struct ActivityQuery {
    hours: Option<f64>,
}

#[derive(Deserialize)]
struct DetectQuery {
    dry_run: Option<bool>,
}

#[derive(Serialize)]
struct ProfileResponse {
    content: String,
}

#[derive(Serialize)]
struct PatternsResponse {
    content: String,
    char_count: usize,
    updated_at: Option<i64>,
}

#[derive(Serialize)]
struct HistoryEntry {
    timestamp: i64,
    size_bytes: u64,
}

#[derive(Serialize)]
struct ShutdownResponse {
    status: &'static str,
}

// ---------- Error type ----------

#[derive(Serialize)]
struct ApiErrorBody {
    code: String,
    message: String,
}

#[derive(Serialize)]
struct ApiErrorEnvelope {
    error: ApiErrorBody,
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: String,
    message: String,
}

impl ApiError {
    fn internal(msg: impl ToString) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "INTERNAL_ERROR".to_string(),
            message: msg.to_string(),
        }
    }

    fn bad_request(msg: impl ToString) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "BAD_REQUEST".to_string(),
            message: msg.to_string(),
        }
    }

    fn not_found(msg: impl ToString) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "NOT_FOUND".to_string(),
            message: msg.to_string(),
        }
    }

    fn conflict(msg: impl ToString) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: "CONFLICT".to_string(),
            message: msg.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let body = ApiErrorEnvelope {
            error: ApiErrorBody {
                code: self.code,
                message: self.message,
            },
        };
        (self.status, Json(body)).into_response()
    }
}

// ---------- Handlers ----------

async fn health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        uptime_s: state.start_time.elapsed().as_secs(),
        daemon_version: state.version,
    })
}

async fn activity(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ActivityQuery>,
) -> Result<Json<Vec<db::EventRow>>, ApiError> {
    let hours = params.hours.unwrap_or(1.0);
    if hours <= 0.0 || !hours.is_finite() {
        return Err(ApiError::bad_request(
            "hours must be a positive finite number",
        ));
    }
    // Cap at 14 days (the prune window) to avoid pointless full-table scans
    let hours = hours.min(336.0);

    let conn = db::open_events_db(&state.data_root.data_dir).map_err(ApiError::internal)?;
    let now = chrono::Utc::now().timestamp_millis();
    let since_ts = now - (hours * 3_600_000.0) as i64;
    let rows = db::query_recent_events(&conn, since_ts).map_err(ApiError::internal)?;

    Ok(Json(rows))
}

#[derive(Deserialize)]
struct RhythmQuery {
    days: Option<u32>,
}

/// GET /api/rhythm?days=7 — focus analytics (windows, fingerprint, drift, heatmap)
/// computed from recent activity events. `days` defaults to 7, clamped to 1..=30.
async fn get_rhythm(
    State(state): State<Arc<AppState>>,
    Query(params): Query<RhythmQuery>,
) -> Result<Json<ccube_core::rhythm::RhythmReport>, ApiError> {
    let days = params.days.unwrap_or(7).clamp(1, 30);
    let conn = db::open_events_db(&state.data_root.data_dir).map_err(ApiError::internal)?;
    let now = chrono::Utc::now().timestamp_millis();
    let since_ts = now - (days as i64) * 24 * 3_600_000;
    let events = db::query_recent_events(&conn, since_ts).map_err(ApiError::internal)?;
    Ok(Json(ccube_core::rhythm::compute_rhythm(&events)))
}

async fn memory_profile(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ProfileResponse>, ApiError> {
    let content = memory::read_profile(&state.data_root.memory_dir).map_err(ApiError::internal)?;
    Ok(Json(ProfileResponse { content }))
}

async fn memory_patterns(
    State(state): State<Arc<AppState>>,
) -> Result<Json<PatternsResponse>, ApiError> {
    let content = memory::read_patterns(&state.data_root.memory_dir).map_err(ApiError::internal)?;
    let char_count = content.len();

    // Get file mtime for updated_at
    let patterns_path = state.data_root.memory_dir.join("patterns.md");
    let updated_at = std::fs::metadata(&patterns_path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|d| d.as_millis() as i64)
        });

    Ok(Json(PatternsResponse {
        content,
        char_count,
        updated_at,
    }))
}

async fn patterns_history(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<HistoryEntry>>, ApiError> {
    let entries = memory::list_history(&state.data_root.memory_dir, "patterns.md")
        .map_err(ApiError::internal)?;

    let result: Vec<HistoryEntry> = entries
        .into_iter()
        .map(|(ts, path)| {
            let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            HistoryEntry {
                timestamp: ts,
                size_bytes,
            }
        })
        .collect();

    Ok(Json(result))
}

async fn shutdown(State(state): State<Arc<AppState>>) -> Json<ShutdownResponse> {
    tracing::info!("shutdown requested via HTTP");
    state.shutdown_token.cancel();
    Json(ShutdownResponse {
        status: "shutting_down",
    })
}

// ---------- Phase 4 handlers ----------

/// GET /briefing — build and return the current briefing.
async fn get_briefing(
    State(state): State<Arc<AppState>>,
) -> Result<Json<briefing::BriefingV2>, ApiError> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let since_ms = now_ms - 3_600_000;

    let conn = db::open_events_db(&state.data_root.data_dir).map_err(ApiError::internal)?;
    let events = db::query_recent_events(&conn, since_ms).map_err(ApiError::internal)?;

    let b = briefing::build_v2(
        now_ms,
        &events,
        &state.frozen_profile,
        &state.frozen_patterns,
        &[],
    );

    Ok(Json(b))
}

/// POST /detect — run v2 two-step detector now, return DetectorV2Output with decision_id.
/// Accepts optional `?dry_run=true` query param to suppress notifications.
async fn detect(
    State(state): State<Arc<AppState>>,
    Query(params): Query<DetectQuery>,
) -> Result<Json<DetectResponse>, ApiError> {
    let start = std::time::Instant::now();
    let now_ms = chrono::Utc::now().timestamp_millis();
    let since_ms = now_ms - 3_600_000;

    let conn = db::open_events_db(&state.data_root.data_dir).map_err(ApiError::internal)?;
    let events = db::query_recent_events(&conn, since_ms).map_err(ApiError::internal)?;

    let briefing = briefing::build_v2(
        now_ms,
        &events,
        &state.frozen_profile,
        &state.frozen_patterns,
        &[],
    );

    let mut output = detector::run_v2(&briefing, state.llm.as_ref()).await;
    let duration_ms = start.elapsed().as_millis() as i64;

    // In dry-run mode, strip the nudge_message so no notification fires
    if params.dry_run.unwrap_or(false) {
        output.nudge_message = None;
    }

    // Persist the decision
    let decision_str = format!("{:?}", output.decision);
    let nudge_style_str = output.nudge_style.as_ref().map(|s| format!("{:?}", s));
    let briefing_json = serde_json::to_string(&briefing)
        .map_err(|e| ApiError::internal(format!("failed to serialize briefing: {e}")))?;

    let decision_id = db::insert_decision(
        &conn,
        now_ms,
        "manual",
        &decision_str,
        &output.reasoning,
        nudge_style_str.as_deref(),
        output.nudge_message.as_deref(),
        &briefing_json,
        &state.frozen_patterns_hash,
        detector::PROMPT_VERSION_V2,
        duration_ms,
    )
    .map_err(ApiError::internal)?;

    Ok(Json(DetectResponse {
        decision_id,
        output,
    }))
}

// ---------- Phase 5 types ----------

#[derive(Serialize, Deserialize)]
pub struct DetectResponse {
    pub decision_id: i64,
    #[serde(flatten)]
    pub output: briefing::DetectorV2Output,
}

#[derive(Deserialize)]
struct CreateCorrectionRequest {
    decision_id: i64,
    verdict: String,
}

#[derive(Deserialize)]
struct CorrectionsQuery {
    status: Option<String>,
    limit: Option<i64>,
}

#[derive(Deserialize)]
struct DecisionsQuery {
    since: Option<i64>,
    limit: Option<i64>,
}

// ---------- Phase 5 handlers ----------

/// POST /corrections — record a user correction for a detector decision.
async fn create_correction(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateCorrectionRequest>,
) -> Result<(StatusCode, Json<db::CorrectionRow>), ApiError> {
    // Look up the decision in events.sqlite
    let events_conn =
        db::open_events_db(&state.data_root.data_dir).map_err(ApiError::internal)?;
    let decision = db::get_decision(&events_conn, body.decision_id)
        .map_err(ApiError::internal)?
        .ok_or_else(|| {
            ApiError::not_found(format!(
                "decision #{} not found (may have been pruned)",
                body.decision_id
            ))
        })?;

    // Insert correction with the decision's full context
    let corr_conn =
        db::open_corrections_db(&state.data_root.data_dir).map_err(ApiError::internal)?;
    let corr_id = db::insert_correction(
        &corr_conn,
        decision.id,
        &decision.decision,
        &body.verdict,
        &decision.briefing_json,
        &decision.patterns_hash,
    )
    .map_err(ApiError::internal)?;

    let row = db::get_correction(&corr_conn, corr_id)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::internal("failed to read back correction"))?;

    Ok((StatusCode::CREATED, Json(row)))
}

/// GET /corrections — list corrections, optionally filtered by status.
async fn list_corrections_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<CorrectionsQuery>,
) -> Result<Json<Vec<db::CorrectionRow>>, ApiError> {
    let limit = params.limit.unwrap_or(50).min(500);
    let pending_only = params.status.as_deref() == Some("pending");

    let conn =
        db::open_corrections_db(&state.data_root.data_dir).map_err(ApiError::internal)?;
    let rows =
        db::list_corrections(&conn, limit, pending_only).map_err(ApiError::internal)?;

    Ok(Json(rows))
}

/// GET /corrections/:id — show a single correction with full context.
async fn get_correction_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<db::CorrectionRow>, ApiError> {
    let conn =
        db::open_corrections_db(&state.data_root.data_dir).map_err(ApiError::internal)?;
    let row = db::get_correction(&conn, id)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found(format!("correction #{id} not found")))?;

    Ok(Json(row))
}

/// GET /decisions — list recent detector decisions.
async fn list_decisions_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<DecisionsQuery>,
) -> Result<Json<Vec<db::DecisionRow>>, ApiError> {
    let since = params.since.unwrap_or(0);
    let limit = params.limit.unwrap_or(50).min(500);

    let conn = db::open_events_db(&state.data_root.data_dir).map_err(ApiError::internal)?;
    let rows = db::list_decisions(&conn, since, limit).map_err(ApiError::internal)?;

    Ok(Json(rows))
}

// ---------- Phase 6: Curator endpoint ----------

#[derive(Deserialize)]
struct CuratorRunQuery {
    dry_run: Option<bool>,
}

#[derive(Serialize)]
pub struct CuratorRunResponse {
    pub trigger: String,
    pub corrections_processed: usize,
    pub correction_verdicts: Vec<briefing::CorrectionVerdict>,
    pub proposed_adds: Vec<briefing::PatternAdd>,
    pub proposed_replaces: Vec<briefing::PatternReplace>,
    pub candidate_patterns: String,
    pub eval_passed: Option<bool>,
    pub committed: bool,
    pub dry_run: bool,
    pub duration_ms: u64,
}

/// POST /agents/curator/run — trigger a curator run manually.
/// Accepts optional `?dry_run=true` to skip eval + write.
async fn run_curator_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<CuratorRunQuery>,
) -> Result<Json<CuratorRunResponse>, ApiError> {
    let dry_run = params.dry_run.unwrap_or(false);

    // Non-blocking try-acquire: if another curator run is in progress, reject.
    let _guard = state
        .curator_mutex
        .try_lock()
        .map_err(|_| ApiError::conflict("curator already running"))?;

    let start = std::time::Instant::now();

    let result = curator::run_curator(
        &state.data_root.data_dir,
        &state.data_root.memory_dir,
        &state.frozen_profile,
        &state.frozen_patterns,
        state.curator_llm.as_ref(),
        state.llm.as_ref(), // eval replay uses detector LLM (10s timeout)
        dry_run,
    )
    .await
    .map_err(ApiError::internal)?;

    let duration_ms = start.elapsed().as_millis() as u64;

    // Log to curator.ndjson
    crate::scheduler::log_curator_run(&state.data_root.logs_dir, "manual", &result, duration_ms);

    Ok(Json(CuratorRunResponse {
        trigger: "manual".to_string(),
        corrections_processed: result.corrections_processed,
        correction_verdicts: result.output.correction_verdicts,
        proposed_adds: result.output.proposed_adds,
        proposed_replaces: result.output.proposed_replaces,
        candidate_patterns: result.candidate_patterns,
        eval_passed: result.eval_result.as_ref().map(|e| e.passed),
        committed: result.committed,
        dry_run: result.dry_run,
        duration_ms,
    }))
}

// ---------- Phase 7: Reflector endpoints ----------

#[derive(Deserialize)]
struct ReflectorRunQuery {
    dry_run: Option<bool>,
}

#[derive(Serialize)]
pub struct ReflectorRunResponse {
    pub trigger: String,
    pub patterns_after: String,
    pub rationale: String,
    pub eval_passed: Option<bool>,
    pub eval_outcome: Option<String>,
    pub committed: bool,
    pub pending: bool,
    pub dry_run: bool,
    pub chars_before: usize,
    pub chars_after: usize,
    pub duration_ms: u64,
}

#[derive(Serialize)]
struct PendingResponse {
    exists: bool,
    content: Option<String>,
    chars: Option<usize>,
}

#[derive(Serialize)]
struct PendingActionResponse {
    status: &'static str,
}

/// POST /agents/reflector/run — trigger a reflector run manually.
async fn run_reflector_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ReflectorRunQuery>,
) -> Result<Json<ReflectorRunResponse>, ApiError> {
    let dry_run = params.dry_run.unwrap_or(false);

    let _guard = state
        .curator_mutex
        .try_lock()
        .map_err(|_| ApiError::conflict("curator or reflector already running"))?;

    let start = std::time::Instant::now();

    // Read live patterns from disk (not frozen)
    let live_patterns =
        memory::read_patterns(&state.data_root.memory_dir).map_err(ApiError::internal)?;

    let result = reflector::run_reflector(
        &state.data_root.data_dir,
        &state.data_root.memory_dir,
        &state.frozen_profile,
        &live_patterns,
        state.curator_llm.as_ref(),
        state.llm.as_ref(),
        dry_run,
    )
    .await
    .map_err(ApiError::internal)?;

    let duration_ms = start.elapsed().as_millis() as u64;

    crate::scheduler::log_reflector_run(
        &state.data_root.logs_dir,
        "manual",
        &result,
        duration_ms,
    );

    let eval_outcome = result.eval_outcome.map(|o| match o {
        ccube_core::eval::ReflectorEvalOutcome::Pass => "pass".to_string(),
        ccube_core::eval::ReflectorEvalOutcome::Borderline => "borderline".to_string(),
        ccube_core::eval::ReflectorEvalOutcome::Fail => "fail".to_string(),
    });

    Ok(Json(ReflectorRunResponse {
        trigger: "manual".to_string(),
        patterns_after: result.patterns_after,
        rationale: result.rationale,
        eval_passed: result.eval_result.as_ref().map(|e| e.passed),
        eval_outcome,
        committed: result.committed,
        pending: result.pending,
        dry_run: result.dry_run,
        chars_before: result.chars_before,
        chars_after: result.chars_after,
        duration_ms,
    }))
}

/// GET /agents/reflector/pending — show pending proposal if any.
async fn get_pending_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<PendingResponse>, ApiError> {
    let content =
        reflector::read_pending(&state.data_root.memory_dir).map_err(ApiError::internal)?;

    Ok(Json(PendingResponse {
        exists: content.is_some(),
        chars: content.as_ref().map(|c| c.len()),
        content,
    }))
}

/// POST /agents/reflector/accept — accept pending proposal.
async fn accept_pending_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<PendingActionResponse>, ApiError> {
    reflector::accept_pending(&state.data_root.memory_dir).map_err(ApiError::internal)?;
    Ok(Json(PendingActionResponse { status: "accepted" }))
}

/// POST /agents/reflector/reject — reject pending proposal.
async fn reject_pending_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<PendingActionResponse>, ApiError> {
    reflector::reject_pending(&state.data_root.memory_dir).map_err(ApiError::internal)?;
    Ok(Json(PendingActionResponse { status: "rejected" }))
}

// ---------- LLM Config endpoints ----------

#[derive(Serialize, Deserialize, Clone)]
pub struct LlmConfig {
    pub provider: String,
    pub url: String,
    pub model: String,
    pub token: Option<String>,
}

#[derive(Serialize)]
struct LlmConfigResponse {
    provider: String,
    url: String,
    model: String,
    has_token: bool,
}

/// GET /config/llm — return current LLM configuration.
async fn get_llm_config(
    State(_state): State<Arc<AppState>>,
) -> Json<LlmConfigResponse> {
    let provider = std::env::var("CCUBE_LLM_PROVIDER")
        .unwrap_or_else(|_| "openai-compatible".to_string());
    let url = std::env::var("CCUBE_LLM_URL")
        .unwrap_or_else(|_| "http://localhost:8080".to_string());
    let model = std::env::var("CCUBE_LLM_MODEL")
        .unwrap_or_else(|_| "default".to_string());
    let has_token = std::env::var("CCUBE_LLM_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
        .is_some();

    Json(LlmConfigResponse {
        provider,
        url,
        model,
        has_token,
    })
}

#[derive(Deserialize)]
struct SetLlmConfigRequest {
    provider: Option<String>,
    url: Option<String>,
    model: Option<String>,
    token: Option<String>,
}

#[derive(Serialize)]
struct SetLlmConfigResponse {
    status: String,
    message: String,
}

/// PUT /config/llm — update LLM configuration in .env file.
async fn set_llm_config(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SetLlmConfigRequest>,
) -> Result<Json<SetLlmConfigResponse>, ApiError> {
    // .env lives in the daemon's working directory (project root)
    let env_path = std::env::current_dir()
        .map_err(ApiError::internal)?
        .join(".env");

    // Read existing .env or create empty
    let existing = if env_path.exists() {
        std::fs::read_to_string(&env_path).map_err(ApiError::internal)?
    } else {
        String::new()
    };

    let mut lines: Vec<String> = existing.lines().map(String::from).collect();

    let mut updated = Vec::new();

    if let Some(ref provider) = body.provider {
        updated.push(("CCUBE_LLM_PROVIDER", provider.clone()));
    }
    if let Some(ref url) = body.url {
        updated.push(("CCUBE_LLM_URL", url.clone()));
    }
    if let Some(ref model) = body.model {
        updated.push(("CCUBE_LLM_MODEL", model.clone()));
    }
    if let Some(ref token) = body.token {
        updated.push(("CCUBE_LLM_TOKEN", token.clone()));
    }

    // Update or add each key
    for (key, value) in &updated {
        let prefix = format!("{}=", key);
        let mut found = false;
        for line in &mut lines {
            if line.starts_with(&prefix) || line.starts_with(&format!("# {}", key)) {
                *line = format!("{}={}", key, value);
                found = true;
                break;
            }
        }
        if !found {
            lines.push(format!("{}={}", key, value));
        }
    }

    let new_content = lines.join("\n");
    std::fs::write(&env_path, new_content).map_err(ApiError::internal)?;

    Ok(Json(SetLlmConfigResponse {
        status: "ok".to_string(),
        message: format!(
            "Updated {} config key(s). Restart daemon to apply.",
            updated.len()
        ),
    }))
}

// ---------- Summarize constants ----------

/// How far back to look for events when summarizing.
const SUMMARIZE_LOOKBACK_HOURS: i64 = 2;

/// Events shorter than this are excluded from grouping (sub-second noise).
const SUMMARIZE_MIN_DURATION_MS: i64 = 0;

/// Maximum tokens for LLM response. Higher to accommodate 200 events with descriptions.
const SUMMARIZE_MAX_TOKENS: u32 = 32768;
/// LLM temperature for summarization.
const SUMMARIZE_TEMPERATURE: f32 = 0.3;

// ---------- Summarize endpoints ----------

/// Extract JSON object from text that may contain reasoning/thinking before it.
fn extract_json(text: &str) -> String {
    // Find the first { and last }
    let start = text.find('{').unwrap_or(0);
    let end = text.rfind('}').map(|i| i + 1).unwrap_or(text.len());
    if start >= end {
        return text.to_string();
    }
    let json = &text[start..end];
    // Try as-is first
    if serde_json::from_str::<serde_json::Value>(json).is_ok() {
        return json.to_string();
    }
    // If invalid, try fixing missing commas between array and next field
    let fixed = json
        .replace("] \"", "], \"")
        .replace(")]\""  , "),\"")
        .replace("}\"", "},\"");
    fixed
}

#[derive(Serialize, Deserialize, Clone)]
struct LlmSessionGroup {
    title: String,
    event_ids: Vec<i64>,
    distraction: bool,
    /// Per-event context descriptions, keyed by event number (1-based).
    /// Maps event_id -> short description of what the user was doing.
    #[serde(default)]
    descriptions: std::collections::HashMap<String, String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SessionGroupWithEvents {
    pub title: String,
    pub distraction: bool,
    pub events: Vec<ccube_core::db::EventRow>,
    pub total_duration_ms: i64,
    #[serde(default)]
    pub event_descriptions: std::collections::HashMap<String, String>,
}

#[derive(Serialize, Clone)]
pub struct SummariesResponse {
    pub generated_at: i64,
    pub groups: Vec<SessionGroupWithEvents>,
}

/// Build the LLM prompt for summarizing events into sessions.
fn build_summarize_prompt(events: &[ccube_core::db::EventRow], corrections: &[String]) -> String {
    let mut lines = Vec::new();
    for (i, event) in events.iter().enumerate() {
        let time = chrono::DateTime::from_timestamp_millis(event.ts)
            .map(|t| t.format("%H:%M").to_string())
            .unwrap_or_default();
        let app = event.app.as_deref().unwrap_or("-");
        let title = event.title.as_deref().unwrap_or("-");
        let dur = event
            .duration_ms
            .map(|d| format!("{}s", d / 1000))
            .unwrap_or_else(|| "?s".to_string());

        // Include OCR text (screen content) when available
        let ocr = event
            .ocr_text
            .as_deref()
            .filter(|t| !t.is_empty())
            .map(|t| {
                let truncated = &t[..t.floor_char_boundary(120)];
                format!(" | Screen: {}", truncated)
            })
            .unwrap_or_default();

        // Include vision description (screen understanding) when available
        let vision = event
            .vision_desc
            .as_deref()
            .filter(|d| !d.is_empty())
            .map(|d| format!(" | Vision: {}", d))
            .unwrap_or_default();

        lines.push(format!(
            "{}. [{}] {} \u{2013} {} ({}){}{}",
            i + 1,
            time,
            app,
            title,
            dur,
            ocr,
            vision
        ));
    }

    let corrections_section = if corrections.is_empty() {
        String::new()
    } else {
        format!(
            "\nRecent user corrections (learn from these):\n{}\n",
            corrections.iter().map(|c| format!("- {}", c)).collect::<Vec<_>>().join("\n")
        )
    };

    format!(
        r#"Analyze these computer activity events and group them into activities.

For each group, provide:
1. A descriptive title (5-10 words) that captures the overall ACTIVITY the user was engaged in
   Good: "Working on history project about World War 2", "Chilling with friends on Discord"
   Bad: "Web Browsing", "Social Chat", "Coding"
2. For EACH event, a short description (3-8 words) of what the user was specifically doing
   Use the app name, window title, screen content, AND vision data to infer specifics
   Good: "watching WWII documentary on YouTube", "writing history essay in Word"
   Bad: "using Brave Browser", "in Microsoft Word"
3. Whether the activity is a distraction (entertainment/aimless browsing) or focused work

Rules:
- Group consecutive events that belong to the same activity
- Use ALL available context: app name, window title, OCR screen text, and vision description
- Include ALL event numbers, do not skip any
- Events are listed newest first
{}Events:
{}

Respond with ONLY a JSON object. Use this exact format:
{{"groups":[{{"title":"Activity Title","event_ids":[1,2],"distraction":false,"descriptions":{{"1":"watching WWII documentary","2":"writing history essay"}}}}]}}

Make sure every field is separated by a comma. Output:
{{"groups":[...]}}"#,
        corrections_section,
        lines.join("\n")
    )
}

/// Core summarization logic — fetches events, calls LLM, parses response into groups.
pub async fn run_summarize(
    state: &AppState,
    since_ms: Option<i64>,
    until_ms: Option<i64>,
) -> Result<SummariesResponse, ApiError> {
    let conn =
        db::open_events_db(&state.data_root.data_dir).map_err(ApiError::internal)?;
    let now = chrono::Utc::now().timestamp_millis();
    let since = since_ms.unwrap_or(now - SUMMARIZE_LOOKBACK_HOURS * 3_600_000);
    let until = until_ms.unwrap_or(i64::MAX);
    let all_events = db::query_recent_events(&conn, since).map_err(ApiError::internal)?;

    // Filter to app_focus events within the time range
    let events: Vec<_> = all_events
        .into_iter()
        .filter(|e| {
            e.kind == "app_focus"
                && e.duration_ms.unwrap_or(0) > SUMMARIZE_MIN_DURATION_MS
                && e.ts < until
        })
        .collect();

    if events.is_empty() {
        return Ok(SummariesResponse {
            generated_at: now,
            groups: vec![],
        });
    }

    // Load recent corrections for prompt context (best-effort)
    let corrections: Vec<String> = db::open_corrections_db(&state.data_root.data_dir)
        .ok()
        .and_then(|conn| {
            db::list_corrections(&conn, 5, false).ok()
        })
        .unwrap_or_default()
        .into_iter()
        .filter_map(|c| {
            if c.user_verdict.starts_with("group_reassign") {
                Some(c.user_verdict)
            } else {
                None
            }
        })
        .collect();

    // Batch events into chunks and call LLM for each batch
    const BATCH_SIZE: usize = 40;
    let mut all_llm_groups: Vec<LlmSessionGroup> = Vec::new();

    for chunk in events.chunks(BATCH_SIZE) {
        let prompt = build_summarize_prompt(chunk, &corrections);

        let response = state
            .llm
            .complete(&prompt, "", SUMMARIZE_MAX_TOKENS, SUMMARIZE_TEMPERATURE)
            .await
            .map_err(|e| ApiError::internal(format!("LLM call failed: {e}")))?;

        let content = response.content.trim();
        let json_str = extract_json(content);

        #[derive(Deserialize)]
        struct SummarizeOutput {
            groups: Vec<LlmSessionGroup>,
        }

        match serde_json::from_str::<SummarizeOutput>(&json_str) {
            Ok(output) => all_llm_groups.extend(output.groups),
            Err(e) => {
                tracing::warn!(error = %e, "Failed to parse LLM batch response, skipping batch");
            }
        }
    }

    // Map LLM groups to SessionGroupWithEvents
    let mut used_event_ids = std::collections::HashSet::<i64>::new();
    let mut groups: Vec<SessionGroupWithEvents> = Vec::new();

    for group in &all_llm_groups {
        let mut group_events = Vec::new();
        let mut mapped_descriptions = std::collections::HashMap::new();

        for &llm_id in &group.event_ids {
            if llm_id < 1 { continue; }
            let idx = (llm_id - 1) as usize;
            if idx < events.len() {
                let event = &events[idx];
                if used_event_ids.insert(event.id) {
                    group_events.push(event.clone());
                }
                if let Some(desc) = group.descriptions.get(&llm_id.to_string()) {
                    mapped_descriptions.insert(event.id.to_string(), desc.clone());
                }
            }
        }

        if group_events.is_empty() { continue; }

        let total_duration: i64 = group_events.iter().filter_map(|e| e.duration_ms).sum();
        groups.push(SessionGroupWithEvents {
            title: group.title.clone(),
            distraction: group.distraction,
            events: group_events,
            total_duration_ms: total_duration,
            event_descriptions: mapped_descriptions,
        });
    }

    // Auto-assign ungrouped events to nearest group by timestamp proximity
    let ungrouped: Vec<_> = events.iter()
        .filter(|e| !used_event_ids.contains(&e.id))
        .collect();

    if !ungrouped.is_empty() && !groups.is_empty() {
        // Build timestamp bounds for each group
        let group_bounds: Vec<(i64, i64)> = groups.iter().map(|g| {
            let ts_vals: Vec<i64> = g.events.iter().map(|e| e.ts).collect();
            (*ts_vals.iter().min().unwrap_or(&0), *ts_vals.iter().max().unwrap_or(&0))
        }).collect();

        for event in ungrouped {
            // Find the group whose time range is closest to this event's timestamp
            let best_idx = group_bounds.iter().enumerate()
                .min_by_key(|(_, (lo, hi))| {
                    if event.ts >= *lo && event.ts <= *hi { 0 }
                    else if event.ts < *lo { lo - event.ts }
                    else { event.ts - hi }
                })
                .map(|(i, _)| i);

            if let Some(idx) = best_idx {
                groups[idx].events.push(event.clone());
                groups[idx].total_duration_ms += event.duration_ms.unwrap_or(0);
                used_event_ids.insert(event.id);
            }
        }

        // Sort events within each group by timestamp
        for g in &mut groups {
            g.events.sort_by_key(|e| e.ts);
        }
    }

    // Fallback: if no groups were formed at all, create one "Activity" group
    if groups.is_empty() && !events.is_empty() {
        let total_duration: i64 = events.iter().filter_map(|e| e.duration_ms).sum();
        groups.push(SessionGroupWithEvents {
            title: "Activity".to_string(),
            distraction: false,
            events,
            total_duration_ms: total_duration,
            event_descriptions: std::collections::HashMap::new(),
        });
    }

    Ok(SummariesResponse {
        generated_at: now,
        groups,
    })
}

/// GET /summaries?range_key=day:2026-05-21 — return persisted summary for a date range.
async fn get_summaries(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SummariesQuery>,
) -> Result<Json<Option<SummariesResponse>>, ApiError> {
    let range_key = match params.range_key {
        Some(k) => k,
        None => {
            // Backward compat: try the in-memory cache
            let cache = state.cached_summaries.read().await;
            return Ok(Json(cache.clone()));
        }
    };

    let conn = db::open_summaries_db(&state.data_root.data_dir).map_err(ApiError::internal)?;
    match db::get_summary(&conn, &range_key).map_err(ApiError::internal)? {
        Some((generated_at, groups_json)) => {
            let groups: Vec<SessionGroupWithEvents> =
                serde_json::from_str(&groups_json).unwrap_or_default();
            Ok(Json(Some(SummariesResponse { generated_at, groups })))
        }
        None => Ok(Json(None)),
    }
}

#[derive(Deserialize)]
struct SummariesQuery {
    range_key: Option<String>,
}

/// POST /summarize — trigger immediate summarization, update cache, return result.
/// Optional JSON body: `{ "since_ms": <epoch_ms>, "until_ms": <epoch_ms> }`
/// If not provided, defaults to last SUMMARIZE_LOOKBACK_HOURS.
async fn run_summarize_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SummarizeRequest>,
) -> Result<Json<SummariesResponse>, ApiError> {
    let result = run_summarize(&state, body.since_ms, body.until_ms).await?;

    // Persist to DB if range_key was provided
    if let Some(rk) = body.range_key.as_deref() {
        let conn = db::open_summaries_db(&state.data_root.data_dir).map_err(ApiError::internal)?;
        let groups_json = serde_json::to_string(&result.groups).unwrap_or_default();
        db::upsert_summary(&conn, rk, body.since_ms.unwrap_or(0), body.until_ms.unwrap_or(i64::MAX), result.generated_at, &groups_json)
            .map_err(ApiError::internal)?;
    }

    *state.cached_summaries.write().await = Some(result.clone());
    Ok(Json(result))
}

#[derive(Deserialize, Default)]
struct SummarizeRequest {
    since_ms: Option<i64>,
    until_ms: Option<i64>,
    range_key: Option<String>,
}

// ---------- Group correction endpoints ----------

#[derive(Deserialize)]
struct GroupCorrectionRequest {
    /// The event ID that was moved
    event_id: i64,
    /// The group title the event was moved FROM
    from_group: String,
    /// The group title the event was moved TO
    to_group: String,
    /// Optional: new title for a renamed group
    renamed_to: Option<String>,
}

#[derive(Serialize)]
struct GroupCorrectionResponse {
    status: String,
    message: String,
}

/// POST /corrections/group — record a user grouping correction.
/// This is the key feedback loop: when the user drags an event between groups
/// or renames a group, we record it for future LLM improvement.
async fn create_group_correction(
    State(state): State<Arc<AppState>>,
    Json(body): Json<GroupCorrectionRequest>,
) -> Result<Json<GroupCorrectionResponse>, ApiError> {
    // Write to corrections DB as a simple record
    let conn =
        db::open_corrections_db(&state.data_root.data_dir).map_err(ApiError::internal)?;

    // Store as a correction with verdict = "group_reassign"
    // decision_id = event_id (reuse field)
    let verdict = format!(
        "group_reassign: event {} from '{}' to '{}'{}",
        body.event_id,
        body.from_group,
        body.to_group,
        body.renamed_to
            .as_ref()
            .map(|r| format!(", renamed to '{}'", r))
            .unwrap_or_default()
    );

    // Use event_id as decision_id (it's just a reference)
    db::insert_correction(
        &conn,
        body.event_id,
        "group_assign",
        &verdict,
        "{}",
        "",
    )
    .map_err(ApiError::internal)?;

    tracing::info!(
        event_id = body.event_id,
        from = %body.from_group,
        to = %body.to_group,
        "group correction recorded"
    );

    Ok(Json(GroupCorrectionResponse {
        status: "ok".to_string(),
        message: format!("Recorded: event {} moved from '{}' to '{}'", body.event_id, body.from_group, body.to_group),
    }))
}
