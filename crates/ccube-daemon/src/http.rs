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
    /// Epoch-ms until which nudge notifications are suppressed (tray snooze).
    /// Detection keeps running and decisions are still recorded; only the
    /// banner is held back. 0 = not snoozed.
    pub snooze_until_ms: Arc<std::sync::atomic::AtomicI64>,
}

impl AppState {
    /// Load the current memory snapshot from disk.
    ///
    /// Each agent run (detector, curator, reflector, briefing) reads a fresh
    /// snapshot so curator/reflector commits and manual `ccube memory edit`s
    /// take effect on the next run without a daemon restart. Replaces the
    /// phase-4 frozen-at-startup memory (see DECISIONS.md 2026-06-10).
    /// Unreadable files degrade to empty memory; agents must never crash.
    pub fn memory_snapshot(&self) -> memory::MemorySnapshot {
        memory::load_snapshot(&self.data_root.memory_dir).unwrap_or_else(|e| {
            tracing::error!(error = %e, "failed to load memory snapshot, using empty memory");
            memory::MemorySnapshot {
                profile: String::new(),
                patterns: String::new(),
                patterns_hash: memory::patterns_hash(""),
            }
        })
    }
}

/// Build the axum router with all endpoints.
pub fn router(state: Arc<AppState>) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/llm/health", get(llm_health))
        .route("/capture/health", get(capture_health))
        .route("/notify/test", post(notify_test))
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
        .route("/sessions/{id}", axum::routing::put(rename_session_handler))
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

/// POST /notify/test — send a sample nudge notification so users can check
/// their notification settings without waiting for a real drift.
/// Deliberately ignores the tray snooze: an explicit "test my setup" click
/// should always produce a banner, or the user can't tell snooze from broken.
async fn notify_test() -> Json<serde_json::Value> {
    crate::notify::send_nudge(0, "Notifications are working — this is what a nudge looks like.");
    Json(serde_json::json!({ "status": "sent" }))
}

/// GET /capture/health — what the capture layer is allowed to see. The UI
/// shows a quiet hint when permissions are missing, because silently blind
/// capture produces mush labels and an undecidable detector.
async fn capture_health() -> Json<serde_json::Value> {
    #[cfg(target_os = "macos")]
    let (accessibility, screen_recording) = (
        ccube_capture::macos::accessibility_permission_now(),
        ccube_capture::macos::screen_permission_now(),
    );
    #[cfg(not(target_os = "macos"))]
    let (accessibility, screen_recording) = (true, true);

    Json(serde_json::json!({
        "accessibility": accessibility,
        "screen_recording": screen_recording,
    }))
}

/// GET /llm/health — probe the configured LLM backend (2s timeout).
/// Lets the UI show a quiet setup hint when Ollama isn't running or the
/// model isn't downloaded, instead of failing silently.
async fn llm_health() -> Result<Json<ccube_core::llm::LlmHealth>, ApiError> {
    let health = tokio::task::spawn_blocking(ccube_core::llm::check_health)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(health))
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

    let mem = state.memory_snapshot();
    let current_activity = db::get_open_session(&conn, &day_range_key(now_ms))
        .ok()
        .flatten()
        .map(|s| s.label);
    let b = briefing::build_v2(now_ms, &events, &mem.profile, &mem.patterns, &[], current_activity);

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

    let mem = state.memory_snapshot();
    let current_activity = db::get_open_session(&conn, &day_range_key(now_ms))
        .ok()
        .flatten()
        .map(|s| s.label);
    let briefing = briefing::build_v2(now_ms, &events, &mem.profile, &mem.patterns, &[], current_activity);

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
        &mem.patterns_hash,
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

    let mem = state.memory_snapshot();
    let result = curator::run_curator(
        &state.data_root.data_dir,
        &state.data_root.memory_dir,
        &mem.profile,
        &mem.patterns,
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

    let mem = state.memory_snapshot();
    let result = reflector::run_reflector(
        &state.data_root.data_dir,
        &state.data_root.memory_dir,
        &mem.profile,
        &mem.patterns,
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
        .unwrap_or_else(|_| "ollama".to_string());
    let url = std::env::var("CCUBE_LLM_URL")
        .unwrap_or_else(|_| "http://localhost:11434/v1".to_string());
    let model = std::env::var("CCUBE_LLM_MODEL")
        .unwrap_or_else(|_| "gemma4:e4b".to_string());
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
    State(_state): State<Arc<AppState>>,
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

/// Events shorter than this are excluded from grouping (sub-second noise).
const SUMMARIZE_MIN_DURATION_MS: i64 = 0;

/// A break only ends an activity when the person was away this long. Short
/// idles (the 5-min idle-detection threshold, for "Away" rows and honest time
/// accounting) do NOT split a session — stepping away for coffee doesn't end
/// "working on companion cube".
const SESSION_AFK_MS: i64 = 20 * 60 * 1000;

/// A raw gap (no idle marker — daemon offline, machine asleep) this long also
/// ends an activity.
const MAX_SESSION_GAP_MS: i64 = SESSION_AFK_MS;

/// Events per holistic segmentation call. Large enough that the model sees
/// the through-line of a work session and groups coarsely (output is just
/// labels + index lists, so it stays reliable); a whole-day segment is
/// chunked at this size and each chunk continues the previous activity.
const SEGMENT_CHUNK: usize = 30;

/// Split a time-ordered event list into segments at break markers
/// (idle_start timestamps) and at long gaps. Sessions never span a segment
/// boundary: whatever follows a break is a new activity by definition, so
/// no LLM judgment is needed — or trusted — across one.
fn split_at_breaks(
    events: Vec<ccube_core::db::EventRow>,
    breaks: &[i64],
    max_gap_ms: i64,
) -> Vec<Vec<ccube_core::db::EventRow>> {
    let mut segments: Vec<Vec<ccube_core::db::EventRow>> = Vec::new();
    let mut current: Vec<ccube_core::db::EventRow> = Vec::new();
    for event in events {
        if let Some(prev) = current.last() {
            let prev_end = prev.ts + prev.duration_ms.unwrap_or(0);
            let break_between = breaks.iter().any(|&b| b > prev.ts && b <= event.ts);
            if break_between || event.ts.saturating_sub(prev_end) > max_gap_ms {
                segments.push(std::mem::take(&mut current));
            }
        }
        current.push(event);
    }
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

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

/// One numbered line of event context for an LLM prompt.
fn format_event_line(n: Option<usize>, event: &ccube_core::db::EventRow) -> String {
    let time = chrono::DateTime::from_timestamp_millis(event.ts)
        .map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_default();
    let app = event.app.as_deref().unwrap_or("-");
    let title = event.title.as_deref().unwrap_or("-");
    let dur = event
        .duration_ms
        .map(|d| format!("{}s", d / 1000))
        .unwrap_or_else(|| "?s".to_string());
    let ocr = event
        .ocr_text
        .as_deref()
        .filter(|t| !t.is_empty())
        .map(|t| format!(" | Screen: {}", &t[..t.floor_char_boundary(200)]))
        .unwrap_or_default();
    let vision = event
        .vision_desc
        .as_deref()
        .filter(|d| !d.is_empty())
        .map(|d| format!(" | Vision: {}", d))
        .unwrap_or_default();
    let prefix = n.map(|n| format!("{n}. ")).unwrap_or_else(|| "- ".to_string());
    format!("{prefix}[{time}] {app} – {title} ({dur}){ocr}{vision}")
}

/// One activity the holistic segmentation carved out of a list of events.
#[derive(Debug, PartialEq, Clone)]
struct Activity {
    label: Option<String>,
    distraction: bool,
    /// 1-based indices into the event list that was sent to the LLM.
    event_idxs: Vec<usize>,
    /// Per-event descriptions, keyed by the same 1-based index.
    descriptions: std::collections::HashMap<usize, String>,
}

/// Parse the holistic segmentation JSON leniently. Ollama ignores grammars and
/// small models emit numbers as strings / skip fields, so we tolerate all of
/// that. Guarantees: activities are ordered; every input event index 1..=n is
/// assigned to exactly one activity (unmentioned events attach to the nearest
/// preceding activity, or a single fallback activity); never panics.
/// One unlabeled activity covering everything — the last resort when the LLM
/// output can't be parsed even after a retry.
fn fallback_activity(n: usize) -> Vec<Activity> {
    vec![Activity {
        label: None,
        distraction: false,
        event_idxs: (1..=n).collect(),
        descriptions: std::collections::HashMap::new(),
    }]
}

/// Lenient parse used at call sites that don't retry (and by tests): returns a
/// fallback single activity when the JSON can't be parsed.
fn parse_segments(json: &str, n: usize) -> Vec<Activity> {
    try_parse_segments(json, n).unwrap_or_else(|| fallback_activity(n))
}

/// Parse the holistic segmentation JSON, or `None` if it is unusable (invalid
/// JSON, or no non-empty `activities`). Distinguishing failure from a model
/// that legitimately returned one activity lets the caller retry instead of
/// silently producing a giant unlabeled blob.
fn try_parse_segments(json: &str, n: usize) -> Option<Vec<Activity>> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let arr = match v.get("activities").and_then(|a| a.as_array()) {
        Some(a) if !a.is_empty() => a,
        _ => return None,
    };

    let as_idx = |v: &serde_json::Value| -> Option<usize> {
        match v {
            serde_json::Value::Number(num) => num.as_i64().map(|n| n as usize),
            serde_json::Value::String(s) => s.trim().parse::<usize>().ok(),
            _ => None,
        }
    };

    let mut activities = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for a in arr {
        let label = a
            .get("label")
            .and_then(|l| l.as_str())
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from);
        let distraction = a
            .get("distraction")
            .map(|d| d.as_bool().unwrap_or(d.as_str() == Some("true")))
            .unwrap_or(false);
        let event_idxs: Vec<usize> = a
            .get("events")
            .and_then(|e| e.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(as_idx)
                    .filter(|&i| (1..=n).contains(&i) && seen.insert(i))
                    .collect()
            })
            .unwrap_or_default();
        let descriptions = a
            .get("descriptions")
            .and_then(|d| d.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, val)| {
                        let idx = k.trim().parse::<usize>().ok()?;
                        let desc = val.as_str()?.trim();
                        ((1..=n).contains(&idx) && !desc.is_empty())
                            .then(|| (idx, desc.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if !event_idxs.is_empty() {
            activities.push(Activity { label, distraction, event_idxs, descriptions });
        }
    }

    if activities.is_empty() {
        return None;
    }

    // Any event the model forgot attaches to the nearest preceding activity
    // (or the first), so no event is ever dropped.
    for i in 1..=n {
        if !seen.contains(&i) {
            let target = activities
                .iter()
                .rposition(|a| a.event_idxs.iter().any(|&x| x < i))
                .unwrap_or(0);
            activities[target].event_idxs.push(i);
        }
    }
    for a in &mut activities {
        a.event_idxs.sort_unstable();
    }
    Some(activities)
}

/// Label contract shared by every grouping prompt. Labels name the *purpose
/// with its subject*, drawn from screen content — never the bare app category.
const LABEL_SPEC: &str = r#"A label names the PURPOSE and its concrete SUBJECT, taken from what is on
screen — not the app name. Use a "<A> while <B>" label when the person is
genuinely doing two things at once.
Good:
  "terminal — working on companion cube"
  "browsing dieter rams design rules and projects"
  "working on history essay while watching history videos and reading papers"
Bad (never do this — these name the tool or activity type, not the subject):
  "mixed work session involving terminal and browsing"
  "web browsing", "using Brave Browser", "general system interaction""#;

/// Holistic segmentation prompt: partition a list of events into coherent
/// activities in one pass. Seeing all events at once (instead of greedy
/// per-batch judgments) is what keeps related work in one activity. When
/// `ongoing` is set, the first events continue that activity unless the
/// person has clearly moved on.
fn build_segment_prompt(
    events: &[ccube_core::db::EventRow],
    ongoing: Option<&str>,
    corrections: &[String],
) -> String {
    let numbered: Vec<String> = events
        .iter()
        .enumerate()
        .map(|(i, e)| format_event_line(Some(i + 1), e))
        .collect();

    let corrections_section = if corrections.is_empty() {
        String::new()
    } else {
        format!(
            "\nThe person has corrected past grouping — learn their preferences:\n{}\n",
            corrections
                .iter()
                .map(|c| format!("- {c}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };

    let ongoing_section = match ongoing {
        Some(label) => format!(
            "\nThese events follow an ongoing activity: \"{label}\". Most of them \
             continue it — only begin a NEW activity when the person clearly \
             shifts to something unrelated and stays there.\n"
        ),
        None => String::new(),
    };

    format!(
        r#"You organize a person's computer activity into a timeline of meaningful activities.

EVENTS (oldest first), each with on-screen text and a vision description:
{events}
{ongoing}
Group consecutive events into a SMALL number of substantial activities —
think 3 to 6 for a whole work session, not dozens. A person working on
something dips into messages, looks things up, and watches a video, then
returns; all of that is ONE activity, named by the main task. A brief
detour (one or two events, a couple of minutes) is part of the activity
around it — never its own activity. Only start a new activity when the
person clearly changes what they are doing and stays there for a while.
When two threads genuinely run together, name them as "<A> while <B>".

{label_spec}
{corrections}
Reply with ONLY this JSON — every event number assigned to exactly one
activity, activities in order (oldest first):
{{"activities": [
  {{"label": "<purpose — subject>", "distraction": <true|false>, "events": [1,2,3]}}
]}}"#,
        events = numbered.join("\n"),
        ongoing = ongoing_section,
        label_spec = LABEL_SPEC,
        corrections = corrections_section,
    )
}

/// Prompt for the final label when a session solidifies.
fn build_solidify_prompt(working_label: &str, events: &[ccube_core::db::EventRow]) -> String {
    let lines: Vec<String> = events
        .iter()
        .map(|e| {
            let desc = e
                .llm_desc
                .as_deref()
                .or(e.vision_desc.as_deref())
                .or(e.title.as_deref())
                .unwrap_or("-");
            let app = e.app.as_deref().unwrap_or("-");
            let mins = e.duration_ms.unwrap_or(0) / 60_000;
            format!("- {app} – {desc} ({mins}m)")
        })
        .collect();
    format!(
        r#"This activity is finished. Give it ONE final name.

{LABEL_SPEC}

Working label so far: "{working_label}"
Everything that happened (oldest first):
{events}

Reply with ONLY: {{"label": "<final name>", "distraction": <true|false>}}"#,
        events = lines.join("\n"),
    )
}

/// Ask the LLM for a finished session's final label. Pure: no DB handle is
/// held across the await (rusqlite's Connection is not Sync, so a borrow
/// alive across an await would make the whole future !Send).
async fn solidify_label(
    llm: &dyn LlmBackend,
    working_label: &str,
    events: &[ccube_core::db::EventRow],
) -> (Option<String>, Option<bool>) {
    if events.is_empty() {
        return (None, None);
    }
    let prompt = build_solidify_prompt(working_label, events);
    match llm.complete(&prompt, "", 512, SUMMARIZE_TEMPERATURE).await {
        Ok(resp) => {
            match serde_json::from_str::<serde_json::Value>(&extract_json(resp.content.trim())) {
                Ok(v) => (
                    v.get("label")
                        .and_then(|l| l.as_str())
                        .map(str::trim)
                        .filter(|l| !l.is_empty())
                        .map(String::from),
                    v.get("distraction").and_then(|d| d.as_bool()),
                ),
                Err(_) => (None, None),
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "solidify label failed; keeping working label");
            (None, None)
        }
    }
}

/// Close a session with the solidify verdict. Label failure never blocks
/// the close — the working label is kept.
fn finish_close(
    conn: &ccube_core::db::Connection,
    session: &ccube_core::db::SessionRow,
    final_label: Option<String>,
    distraction: Option<bool>,
) {
    if let Err(e) = db::close_session(conn, session.id, final_label.as_deref()) {
        tracing::error!(error = %e, session_id = session.id, "failed to close session");
        return;
    }
    if let Some(d) = distraction {
        let _ = conn.execute(
            "UPDATE sessions SET distraction = ?2 WHERE id = ?1 AND pinned = 0",
            (session.id, d),
        );
    }
    tracing::info!(
        session_id = session.id,
        label = %final_label.as_deref().unwrap_or(&session.label),
        "session solidified"
    );
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SessionGroupWithEvents {
    /// Stable session ID (sessions table). Corrections and renames reference
    /// this, never the title.
    pub id: i64,
    pub title: String,
    pub distraction: bool,
    /// User-touched sessions are pinned; organize passes never modify them.
    pub pinned: bool,
    /// Still absorbing new events — the live head of the timeline.
    pub open: bool,
    pub events: Vec<ccube_core::db::EventRow>,
    pub total_duration_ms: i64,
}

#[derive(Serialize, Clone)]
pub struct SummariesResponse {
    pub generated_at: i64,
    pub groups: Vec<SessionGroupWithEvents>,
}

/// Canonical day range key for a timestamp, in local time.
pub(crate) fn day_range_key(ts_ms: i64) -> String {
    let dt = chrono::DateTime::from_timestamp_millis(ts_ms)
        .map(|t| t.with_timezone(&chrono::Local))
        .unwrap_or_else(chrono::Local::now);
    format!("day:{}", dt.format("%Y-%m-%d"))
}

/// Local-midnight start of the day containing `ts_ms`.
fn day_start_ms(ts_ms: i64) -> i64 {
    use chrono::TimeZone;
    let dt = chrono::DateTime::from_timestamp_millis(ts_ms)
        .map(|t| t.with_timezone(&chrono::Local))
        .unwrap_or_else(chrono::Local::now);
    chrono::Local
        .with_ymd_and_hms(
            chrono::Datelike::year(&dt),
            chrono::Datelike::month(&dt),
            chrono::Datelike::day(&dt),
            0,
            0,
            0,
        )
        .single()
        .map(|d| d.timestamp_millis())
        .unwrap_or(ts_ms)
}

/// Assemble the current state of a range from the sessions table.
fn load_sessions_response(
    conn: &ccube_core::db::Connection,
    range_key: &str,
) -> Result<SummariesResponse, ApiError> {
    let sessions = db::list_sessions(conn, range_key).map_err(ApiError::internal)?;
    let mut groups = Vec::with_capacity(sessions.len());
    for s in sessions {
        let events = db::query_events_by_session(conn, s.id).map_err(ApiError::internal)?;
        if events.is_empty() {
            continue; // pinned-but-empty sessions don't render
        }
        let total_duration_ms = events.iter().filter_map(|e| e.duration_ms).sum();
        groups.push(SessionGroupWithEvents {
            id: s.id,
            title: s.label,
            distraction: s.distraction,
            pinned: s.pinned,
            open: s.open,
            events,
            total_duration_ms,
        });
    }
    Ok(SummariesResponse {
        generated_at: chrono::Utc::now().timestamp_millis(),
        groups,
    })
}

/// Core summarization: holistic re-segmentation of the unsolidified tail.
///
/// One algorithm for both the 5-minute auto pass and ⚡ Organize. Solidified
/// sessions are frozen; the *open* session and any newer ungrouped events form
/// the tail. The tail is split only on long AFK / large gaps, and each segment
/// is partitioned into coherent activities by a single holistic LLM call
/// (seeing all its events at once — the cure for greedy over-fragmentation).
/// `full = true` first clears unpinned sessions so the whole range is rebuilt;
/// pinned (user-edited) sessions are never touched.
pub async fn run_summarize(
    state: &AppState,
    since_ms: Option<i64>,
    until_ms: Option<i64>,
    range_key: Option<String>,
    full: bool,
) -> Result<SummariesResponse, ApiError> {
    let conn = db::open_events_db(&state.data_root.data_dir).map_err(ApiError::internal)?;
    let now = chrono::Utc::now().timestamp_millis();
    let range_key = range_key.unwrap_or_else(|| day_range_key(now));
    let is_today = range_key == day_range_key(now);
    let since = since_ms.unwrap_or_else(|| day_start_ms(now));
    let until = until_ms.unwrap_or(i64::MAX);

    db::close_stale_open_sessions(&conn, &range_key).map_err(ApiError::internal)?;
    if full {
        let cleared = db::clear_unpinned_sessions(&conn, &range_key).map_err(ApiError::internal)?;
        tracing::info!(range_key, cleared, "organize: cleared unpinned sessions");
    }

    let all_events = db::query_recent_events(&conn, since).map_err(ApiError::internal)?;

    // Only AFK ≥ SESSION_AFK_MS ends an activity. The away length is stored on
    // the idle_start row (see main.rs AFK accounting); a 5-min coffee idle is
    // below the bar and stays inside the session.
    let long_breaks: Vec<i64> = all_events
        .iter()
        .filter(|e| e.kind == "idle_start" && e.duration_ms.unwrap_or(0) >= SESSION_AFK_MS)
        .map(|e| e.ts)
        .collect();

    // The prior open session, if any. A long break after its last event means
    // the activity is over — solidify it (refine the final label) before
    // re-segmenting what follows.
    let mut open = db::get_open_session(&conn, &range_key).map_err(ApiError::internal)?;
    if let Some(ref s) = open
        && !s.pinned
        && long_breaks.iter().any(|&b| b > s.end_ts)
    {
        let members = db::query_events_by_session(&conn, s.id).unwrap_or_default();
        let (label, distraction) = solidify_label(state.llm.as_ref(), &s.label, &members).await;
        finish_close(&conn, s, label, distraction);
        open = None;
    }

    // The tail to (re)group: every ungrouped event in range. After `full` this
    // is the whole range; otherwise it is the events captured since last pass.
    let ungrouped: Vec<_> = all_events
        .into_iter()
        .filter(|e| {
            e.kind == "app_focus"
                && e.duration_ms.unwrap_or(0) > SUMMARIZE_MIN_DURATION_MS
                && e.ts < until
                && e.session_id.is_none()
        })
        .collect();

    if ungrouped.is_empty() {
        return load_sessions_response(&conn, &range_key);
    }

    // Correction context: moves teach membership, renames teach naming taste.
    let corrections: Vec<String> = db::open_corrections_db(&state.data_root.data_dir)
        .ok()
        .and_then(|c| db::list_corrections(&c, 5, false).ok())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|c| {
            (c.user_verdict.starts_with("group_reassign")
                || c.user_verdict.starts_with("group_rename"))
            .then_some(c.user_verdict)
        })
        .collect();

    let segments = split_at_breaks(ungrouped, &long_breaks, MAX_SESSION_GAP_MS);

    for (si, segment) in segments.iter().enumerate() {
        // The prior open session only continues into the first segment; any
        // segment after a long break starts a fresh activity.
        let mut continue_id: Option<i64> = if si == 0 { open.as_ref().map(|s| s.id) } else { None };
        let mut ongoing_label: Option<String> =
            if si == 0 { open.as_ref().map(|s| s.label.clone()) } else { None };

        for chunk in segment.chunks(SEGMENT_CHUNK) {
            let prompt = build_segment_prompt(chunk, ongoing_label.as_deref(), &corrections);
            // One holistic call, with a single retry on unparseable output —
            // a transient ramble shouldn't collapse a whole chunk into one
            // unlabeled blob. Only on a second failure do we fall back.
            let mut activities: Option<Vec<Activity>> = None;
            for attempt in 0..2 {
                match state
                    .llm
                    .complete(&prompt, "", SUMMARIZE_MAX_TOKENS, SUMMARIZE_TEMPERATURE)
                    .await
                {
                    Ok(resp) => {
                        activities = try_parse_segments(&extract_json(resp.content.trim()), chunk.len());
                        if activities.is_some() {
                            break;
                        }
                        tracing::warn!(attempt, "segmentation output unparseable; retrying");
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, attempt, "segmentation call failed");
                    }
                }
            }
            let activities = activities.unwrap_or_else(|| fallback_activity(chunk.len()));

            // Persist this chunk atomically (scoped so the non-Send tx is
            // dropped before the next await).
            let last_sid = {
                let tx = conn.unchecked_transaction().map_err(ApiError::internal)?;
                let mut last_sid: Option<i64> = None;
                for (ai, act) in activities.iter().enumerate() {
                    let label = act.label.as_deref().unwrap_or("Activity");
                    // The first activity of the first chunk extends the
                    // session we're continuing; everything else is new.
                    let sid = if ai == 0 && continue_id.is_some() {
                        let id = continue_id.unwrap();
                        if act.label.is_some() {
                            db::update_open_label(&tx, id, label).map_err(ApiError::internal)?;
                        }
                        let _ = tx.execute(
                            "UPDATE sessions SET distraction = ?2 WHERE id = ?1 AND pinned = 0",
                            (id, act.distraction),
                        );
                        id
                    } else {
                        let lo = act.event_idxs.first().and_then(|&i| chunk.get(i - 1)).map(|e| e.ts).unwrap_or(now);
                        let hi = act.event_idxs.last().and_then(|&i| chunk.get(i - 1)).map(|e| e.ts).unwrap_or(now);
                        db::create_session(&tx, &range_key, label, lo, hi, act.distraction, "llm", false)
                            .map_err(ApiError::internal)?
                    };
                    for &i in &act.event_idxs {
                        if let Some(e) = chunk.get(i - 1) {
                            db::assign_event_session(&tx, e.id, Some(sid)).map_err(ApiError::internal)?;
                            if let Some(desc) = act.descriptions.get(&i) {
                                db::update_event_llm_desc(&tx, e.id, desc).map_err(ApiError::internal)?;
                            }
                        }
                    }
                    last_sid = Some(sid);
                }
                tx.commit().map_err(ApiError::internal)?;
                last_sid
            };

            // The next chunk continues the last activity of this one.
            continue_id = last_sid;
            ongoing_label = activities.last().and_then(|a| a.label.clone());
        }
    }

    // Exactly one open session per day: the latest unpinned one, and only for
    // today (past days are fully solidified). Bulletproof regardless of path.
    conn.execute(
        "UPDATE sessions SET open = 0 WHERE range_key = ?1 AND pinned = 0",
        [&range_key],
    )
    .map_err(ApiError::internal)?;
    if is_today {
        conn.execute(
            "UPDATE sessions SET open = 1 WHERE id = (
                 SELECT id FROM sessions
                 WHERE range_key = ?1 AND pinned = 0
                   AND id IN (SELECT DISTINCT session_id FROM events WHERE session_id IS NOT NULL)
                 ORDER BY end_ts DESC LIMIT 1)",
            [&range_key],
        )
        .map_err(ApiError::internal)?;
    }

    load_sessions_response(&conn, &range_key)
}

/// GET /summaries?range_key=day:2026-05-21 — current sessions for a range,
/// read straight from the sessions table (no LLM call).
async fn get_summaries(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SummariesQuery>,
) -> Result<Json<Option<SummariesResponse>>, ApiError> {
    let range_key = match params.range_key {
        Some(k) => k,
        None => {
            // Backward compat: the in-memory cache from the auto-loop
            let cache = state.cached_summaries.read().await;
            return Ok(Json(cache.clone()));
        }
    };

    let conn = db::open_events_db(&state.data_root.data_dir).map_err(ApiError::internal)?;
    let resp = load_sessions_response(&conn, &range_key)?;
    if resp.groups.is_empty() {
        return Ok(Json(None));
    }
    Ok(Json(Some(resp)))
}

#[derive(Deserialize)]
struct SummariesQuery {
    range_key: Option<String>,
}

/// POST /summarize — run an organize pass and return the resulting sessions.
/// Body: `{ since_ms?, until_ms?, range_key?, full? }`. `full: true` (the ⚡
/// Organize button) regroups the whole range except pinned sessions;
/// otherwise only ungrouped events are summarized.
async fn run_summarize_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SummarizeRequest>,
) -> Result<Json<SummariesResponse>, ApiError> {
    let result = run_summarize(
        &state,
        body.since_ms,
        body.until_ms,
        body.range_key,
        body.full.unwrap_or(false),
    )
    .await?;

    *state.cached_summaries.write().await = Some(result.clone());
    Ok(Json(result))
}

#[derive(Deserialize, Default)]
struct SummarizeRequest {
    since_ms: Option<i64>,
    until_ms: Option<i64>,
    range_key: Option<String>,
    full: Option<bool>,
}

// ---------- Group correction endpoints ----------

#[derive(Deserialize)]
struct GroupCorrectionRequest {
    /// The event ID that was moved
    event_id: i64,
    /// Destination session. None = create a new session for the event.
    to_session_id: Option<i64>,
    /// Label for a newly created session (used when to_session_id is None).
    new_session_label: Option<String>,
    /// false suppresses the correction record (used by undo).
    record: Option<bool>,
}

#[derive(Serialize)]
struct GroupCorrectionResponse {
    status: String,
    /// The session the event ended up in (new or existing).
    session_id: i64,
}

/// POST /corrections/group — move an event between sessions.
///
/// This is the key feedback loop. The move is applied to the sessions table
/// immediately (source + destination both pin: the user curated them), and a
/// correction record teaches the curator. The next organize pass cannot undo
/// the move because pinned sessions are off-limits.
async fn create_group_correction(
    State(state): State<Arc<AppState>>,
    Json(body): Json<GroupCorrectionRequest>,
) -> Result<Json<GroupCorrectionResponse>, ApiError> {
    let conn = db::open_events_db(&state.data_root.data_dir).map_err(ApiError::internal)?;

    // Resolve context before mutating, for a readable correction record.
    let event = db::get_event(&conn, body.event_id)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::bad_request("unknown event_id"))?;
    let from_session = match event.session_id {
        Some(sid) => db::get_session(&conn, sid).map_err(ApiError::internal)?,
        None => None,
    };

    let to_session_id = match body.to_session_id {
        Some(sid) => {
            let s = db::get_session(&conn, sid)
                .map_err(ApiError::internal)?
                .ok_or_else(|| ApiError::bad_request("unknown to_session_id"))?;
            db::set_session_pinned(&conn, s.id, true).map_err(ApiError::internal)?;
            s.id
        }
        None => {
            let label = body
                .new_session_label
                .as_deref()
                .unwrap_or("New session");
            // created_by=user → born pinned
            db::create_session(
                &conn,
                &day_range_key(event.ts),
                label,
                event.ts,
                event.ts,
                false,
                "user",
                false,
            )
            .map_err(ApiError::internal)?
        }
    };

    db::assign_event_session(&conn, body.event_id, Some(to_session_id))
        .map_err(ApiError::internal)?;
    // Pin the source too (if it survived losing the event): its membership
    // is now user-curated.
    if let Some(ref from) = from_session
        && db::get_session(&conn, from.id).map_err(ApiError::internal)?.is_some()
    {
        db::set_session_pinned(&conn, from.id, true).map_err(ApiError::internal)?;
    }

    if body.record.unwrap_or(true) {
        let to_label = db::get_session(&conn, to_session_id)
            .map_err(ApiError::internal)?
            .map(|s| s.label)
            .unwrap_or_default();
        let verdict = format!(
            "group_reassign: event {} ({} – {}) from '{}' to '{}'",
            body.event_id,
            event.app.as_deref().unwrap_or("?"),
            event.llm_desc.as_deref().or(event.title.as_deref()).unwrap_or("?"),
            from_session.as_ref().map(|s| s.label.as_str()).unwrap_or("(ungrouped)"),
            to_label,
        );
        let corr_conn =
            db::open_corrections_db(&state.data_root.data_dir).map_err(ApiError::internal)?;
        db::insert_correction(&corr_conn, body.event_id, "group_assign", &verdict, "{}", "")
            .map_err(ApiError::internal)?;
        tracing::info!(event_id = body.event_id, to_session_id, %verdict, "group correction recorded");
    }

    Ok(Json(GroupCorrectionResponse {
        status: "ok".to_string(),
        session_id: to_session_id,
    }))
}

#[derive(Deserialize)]
struct RenameSessionRequest {
    label: String,
}

/// PUT /sessions/{id} — rename a session. Renames pin (the LLM never
/// overwrites a user's label) and are recorded for the curator.
async fn rename_session_handler(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<i64>,
    Json(body): Json<RenameSessionRequest>,
) -> Result<Json<GroupCorrectionResponse>, ApiError> {
    let conn = db::open_events_db(&state.data_root.data_dir).map_err(ApiError::internal)?;
    let old = db::get_session(&conn, id)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::bad_request("unknown session id"))?;

    if !db::rename_session(&conn, id, &body.label).map_err(ApiError::internal)? {
        return Err(ApiError::internal("rename failed"));
    }

    let verdict = format!("group_rename: '{}' renamed to '{}'", old.label, body.label);
    let corr_conn =
        db::open_corrections_db(&state.data_root.data_dir).map_err(ApiError::internal)?;
    db::insert_correction(&corr_conn, id, "group_rename", &verdict, "{}", "")
        .map_err(ApiError::internal)?;
    tracing::info!(session_id = id, %verdict, "session renamed");

    Ok(Json(GroupCorrectionResponse {
        status: "ok".to_string(),
        session_id: id,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(id: i64, ts: i64, dur: i64) -> ccube_core::db::EventRow {
        ccube_core::db::EventRow {
            id,
            ts,
            kind: "app_focus".to_string(),
            app: Some("App".to_string()),
            title: None,
            duration_ms: Some(dur),
            mode: None,
            ocr_text: None,
            vision_desc: None,
            session_id: None,
            llm_desc: None,
        }
    }

    #[test]
    fn segments_lenient_parse_and_full_coverage() {
        // string indices, an out-of-range index, an empty description, and
        // event 3 deliberately omitted from any activity.
        let acts = parse_segments(
            r#"{"activities":[
                {"label":" terminal — working on companion cube ","distraction":false,
                 "events":["1","2"],"descriptions":{"1":"editing rust code","9":"oob","2":""}},
                {"label":"checking discord","distraction":true,"events":[4]}
            ]}"#,
            4,
        );
        assert_eq!(acts.len(), 2);
        assert_eq!(acts[0].label.as_deref(), Some("terminal — working on companion cube"));
        assert!(!acts[0].distraction);
        assert_eq!(acts[1].distraction, true);
        assert_eq!(acts[0].descriptions.get(&1).map(String::as_str), Some("editing rust code"));
        assert!(acts[0].descriptions.get(&2).is_none()); // empty dropped
        // event 3 (omitted) attaches to the nearest preceding activity (#0).
        assert!(acts[0].event_idxs.contains(&3));
        // every input event is assigned exactly once.
        let mut all: Vec<usize> = acts.iter().flat_map(|a| a.event_idxs.clone()).collect();
        all.sort_unstable();
        assert_eq!(all, vec![1, 2, 3, 4]);
    }

    #[test]
    fn segments_fallback_is_one_activity() {
        // garbage / empty → a single activity covering everything, never empty.
        for bad in ["", "not json", r#"{"activities":[]}"#] {
            let acts = parse_segments(bad, 3);
            assert_eq!(acts.len(), 1, "input {bad:?}");
            assert_eq!(acts[0].event_idxs, vec![1, 2, 3]);
            assert!(acts[0].label.is_none());
        }
    }

    #[test]
    fn split_contiguous_stays_whole() {
        let segs = split_at_breaks(vec![ev(1, 0, 1000), ev(2, 1000, 1000)], &[], 900_000);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].len(), 2);
    }

    #[test]
    fn split_at_idle_marker() {
        // idle began at ts 5000, between the two events
        let segs = split_at_breaks(
            vec![ev(1, 0, 1000), ev(2, 600_000, 1000)],
            &[5000],
            i64::MAX,
        );
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0][0].id, 1);
        assert_eq!(segs[1][0].id, 2);
    }

    #[test]
    fn split_at_long_gap_without_marker() {
        // 20-minute hole (daemon off / suspend), no idle row
        let segs = split_at_breaks(
            vec![ev(1, 0, 1000), ev(2, 1_201_000, 1000)],
            &[],
            900_000,
        );
        assert_eq!(segs.len(), 2);
    }

    #[test]
    fn split_ignores_breaks_outside_range() {
        // idle marker before the first event must not split anything
        let segs = split_at_breaks(
            vec![ev(1, 10_000, 1000), ev(2, 12_000, 1000)],
            &[5000],
            900_000,
        );
        assert_eq!(segs.len(), 1);
    }

    #[test]
    fn only_long_idles_break_sessions() {
        // An idle_start row carries the away length in duration_ms. The caller
        // keeps only those ≥ SESSION_AFK_MS as session breaks; a 5-min coffee
        // idle is below the bar and must not split "working on X".
        let idle = |ts: i64, away: i64| ccube_core::db::EventRow {
            kind: "idle_start".to_string(),
            duration_ms: Some(away),
            ..ev(0, ts, 0)
        };
        let rows = [idle(100, 5 * 60_000), idle(200, 25 * 60_000)];
        let long: Vec<i64> = rows
            .iter()
            .filter(|e| e.kind == "idle_start" && e.duration_ms.unwrap_or(0) >= SESSION_AFK_MS)
            .map(|e| e.ts)
            .collect();
        assert_eq!(long, vec![200], "only the 25-min idle is a session break");
    }
}
