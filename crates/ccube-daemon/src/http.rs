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

/// A gap longer than this between consecutive events starts a new segment
/// even without an idle marker (daemon offline, machine asleep).
const MAX_SESSION_GAP_MS: i64 = 15 * 60 * 1000;

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
        .map(|t| format!(" | Screen: {}", &t[..t.floor_char_boundary(120)]))
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

/// The LLM's verdict on a batch of new events vs. the open session.
#[derive(Debug, PartialEq)]
struct MembershipDecision {
    /// Events 1..=k of the batch belong to the current activity.
    continue_through: usize,
    /// Refreshed (or, for a new session, initial) activity label.
    label: Option<String>,
    distraction: bool,
    /// Per-event descriptions, keyed by 1-based batch index.
    descriptions: Vec<(usize, String)>,
}

impl MembershipDecision {
    /// Fallback when the LLM is unreachable or unparseable: absorb the whole
    /// batch silently rather than re-sending it every five minutes.
    fn absorb_all(batch_len: usize) -> Self {
        Self {
            continue_through: batch_len,
            label: None,
            distraction: false,
            descriptions: Vec::new(),
        }
    }
}

/// Parse the membership JSON leniently (Ollama ignores grammars; small
/// models emit numbers as strings and skip fields).
fn parse_membership(json: &str, batch_len: usize, has_open: bool) -> MembershipDecision {
    let v: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => return MembershipDecision::absorb_all(batch_len),
    };

    let as_usize = |v: &serde_json::Value| -> Option<usize> {
        match v {
            serde_json::Value::Number(n) => n.as_i64().map(|n| n.max(0) as usize),
            serde_json::Value::String(s) => s.trim().parse::<i64>().ok().map(|n| n.max(0) as usize),
            _ => None,
        }
    };

    let mut k = v
        .get("continue_through")
        .and_then(as_usize)
        .unwrap_or(batch_len)
        .min(batch_len);
    // A fresh activity owns at least its first event, or we'd never progress.
    if !has_open && k == 0 {
        k = 1;
    }

    let label = v
        .get("label")
        .and_then(|l| l.as_str())
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from);

    let distraction = v
        .get("distraction")
        .map(|d| d.as_bool().unwrap_or(d.as_str() == Some("true")))
        .unwrap_or(false);

    let descriptions = v
        .get("descriptions")
        .and_then(|d| d.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(key, val)| {
                    let idx = key.trim().parse::<usize>().ok()?;
                    let desc = val.as_str()?.trim();
                    ((1..=batch_len).contains(&idx) && !desc.is_empty())
                        .then(|| (idx, desc.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();

    MembershipDecision {
        continue_through: k,
        label,
        distraction,
        descriptions,
    }
}

const LABEL_EXAMPLES: &str = r#""terminal — working on companion cube"
"browsing dieter rams design rules and projects"
"working on history essay while watching history videos and reading papers""#;

/// Prompt asking where the current activity ends within a batch of new events.
fn build_membership_prompt(
    open: Option<(&str, &[ccube_core::db::EventRow])>,
    batch: &[ccube_core::db::EventRow],
    corrections: &[String],
) -> String {
    let numbered: Vec<String> = batch
        .iter()
        .enumerate()
        .map(|(i, e)| format_event_line(Some(i + 1), e))
        .collect();

    let corrections_section = if corrections.is_empty() {
        String::new()
    } else {
        format!(
            "\nThe user has corrected past grouping — learn from these:\n{}\n",
            corrections
                .iter()
                .map(|c| format!("- {c}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };

    match open {
        Some((label, tail)) => {
            let tail_lines: Vec<String> =
                tail.iter().map(|e| format_event_line(None, e)).collect();
            format!(
                r#"You maintain a person's computer activity timeline by deciding where one activity ends and the next begins.

CURRENT ACTIVITY: "{label}"
Its latest events:
{tail}

NEW EVENTS (oldest first):
{events}

How many of the new events (counting from 1) still belong to the current activity? People interleave: quick message checks, reference lookups, and videos that serve the same purpose all stay in ONE activity. It ends only when the person clearly moves on to something unrelated.
{corrections}
Reply with ONLY this JSON:
{{"continue_through": <number of the last belonging event, 0 if even the first starts something new>,
 "label": "<improved label for the current activity, given what you now know>",
 "distraction": <true if it is entertainment or aimless browsing>,
 "descriptions": {{"1": "<3-8 specific words>", "2": "..."}}}}

Labels name the purpose with its context, like:
{examples}"#,
                label = label,
                tail = tail_lines.join("\n"),
                events = numbered.join("\n"),
                corrections = corrections_section,
                examples = LABEL_EXAMPLES,
            )
        }
        None => format!(
            r#"You maintain a person's computer activity timeline by deciding where one activity ends and the next begins.

EVENTS (oldest first):
{events}

These begin a new activity. How many of them (counting from 1) belong to that first activity, before the person moves on to something different? People interleave: quick message checks, reference lookups, and videos that serve the same purpose all stay in ONE activity.
{corrections}
Reply with ONLY this JSON:
{{"continue_through": <number of the last event still in the first activity>,
 "label": "<specific label for the first activity>",
 "distraction": <true if it is entertainment or aimless browsing>,
 "descriptions": {{"1": "<3-8 specific words>", "2": "..."}}}}

Labels name the purpose with its context, like:
{examples}"#,
            events = numbered.join("\n"),
            corrections = corrections_section,
            examples = LABEL_EXAMPLES,
        ),
    }
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
        r#"This activity is finished. Name it in ONE specific phrase: the purpose with its context, like:
{LABEL_EXAMPLES}

Working label: "{working_label}"
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
            "{}. [{}] {} – {} ({}){}{}",
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
- Events are listed oldest first
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

/// Core summarization logic.
///
/// The LLM proposes; the sessions table is the source of truth. An
/// incremental pass (`full = false`, the 5-minute auto-loop) only groups
/// events that belong to no session yet — it appends, never rewrites. A full
/// pass (`full = true`, the ⚡ Organize button) clears *unpinned* sessions in
/// the range and regroups, but never touches pinned (user-edited) sessions
/// or their events.
pub async fn run_summarize(
    state: &AppState,
    since_ms: Option<i64>,
    until_ms: Option<i64>,
    range_key: Option<String>,
    full: bool,
) -> Result<SummariesResponse, ApiError> {
    let conn =
        db::open_events_db(&state.data_root.data_dir).map_err(ApiError::internal)?;
    let now = chrono::Utc::now().timestamp_millis();
    let range_key = range_key.unwrap_or_else(|| day_range_key(now));
    let since = since_ms.unwrap_or_else(|| day_start_ms(now));
    let until = until_ms.unwrap_or(i64::MAX);

    if full {
        let cleared =
            db::clear_unpinned_sessions(&conn, &range_key).map_err(ApiError::internal)?;
        tracing::info!(range_key, cleared, "organize: cleared unpinned sessions");
    }

    let all_events = db::query_recent_events(&conn, since).map_err(ApiError::internal)?;

    // Away periods and long gaps are hard session boundaries: whatever the
    // user does after lunch is a new session, no matter how similar. Collect
    // break markers before filtering down to groupable rows.
    let break_ts: Vec<i64> = all_events
        .iter()
        .filter(|e| e.kind == "idle_start")
        .map(|e| e.ts)
        .collect();

    // Only unassigned app_focus events get grouped; events in sessions
    // (pinned or fresh) are already owned.
    let events: Vec<_> = all_events
        .into_iter()
        .filter(|e| {
            e.kind == "app_focus"
                && e.duration_ms.unwrap_or(0) > SUMMARIZE_MIN_DURATION_MS
                && e.ts < until
                && e.session_id.is_none()
        })
        .collect();

    // Lifecycle that must run even when no new events arrived:
    // day rollover, and an AFK/sleep break after the open session's last
    // event solidifies it (the user left; the activity is over).
    db::close_stale_open_sessions(&conn, &range_key).map_err(ApiError::internal)?;
    let mut open = db::get_open_session(&conn, &range_key).map_err(ApiError::internal)?;
    if let Some(ref s) = open
        && break_ts.iter().any(|&b| b > s.end_ts)
    {
        let members = db::query_events_by_session(&conn, s.id).unwrap_or_default();
        let (label, distraction) = solidify_label(state.llm.as_ref(), &s.label, &members).await;
        finish_close(&conn, s, if s.pinned { None } else { label }, distraction);
        open = None;
    }

    if events.is_empty() {
        return load_sessions_response(&conn, &range_key);
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
            // Both moves and renames teach the grouping LLM: reassigns show
            // which events belong together, renames show the user's naming
            // taste for future titles.
            if c.user_verdict.starts_with("group_reassign")
                || c.user_verdict.starts_with("group_rename")
            {
                Some(c.user_verdict)
            } else {
                None
            }
        })
        .collect();

    const BATCH_SIZE: usize = 40;

    let segments = split_at_breaks(events, &break_ts, MAX_SESSION_GAP_MS);

    for segment in &segments {
        // The open session only survives into a segment it is contiguous
        // with: any break marker or long gap in between solidifies it.
        if let Some(ref s) = open {
            let first_ts = segment.first().map(|e| e.ts).unwrap_or(now);
            let break_between = break_ts.iter().any(|&b| b > s.end_ts && b <= first_ts);
            if break_between || first_ts.saturating_sub(s.end_ts) > MAX_SESSION_GAP_MS {
                let members = db::query_events_by_session(&conn, s.id).unwrap_or_default();
                let (label, distraction) =
                    solidify_label(state.llm.as_ref(), &s.label, &members).await;
                finish_close(&conn, s, if s.pinned { None } else { label }, distraction);
                open = None;
            }
        }

        let mut idx = 0usize;
        while idx < segment.len() {
            let batch = &segment[idx..(idx + BATCH_SIZE).min(segment.len())];

            // Context for the judgment: the open session's label and its
            // most recent members.
            let open_tail: Option<(String, Vec<ccube_core::db::EventRow>)> = match open {
                Some(ref s) => {
                    let members =
                        db::query_events_by_session(&conn, s.id).map_err(ApiError::internal)?;
                    let tail: Vec<_> = members.into_iter().rev().take(5).rev().collect();
                    Some((s.label.clone(), tail))
                }
                None => None,
            };

            let prompt = build_membership_prompt(
                open_tail.as_ref().map(|(l, t)| (l.as_str(), t.as_slice())),
                batch,
                &corrections,
            );
            let decision = match state
                .llm
                .complete(&prompt, "", 4096, SUMMARIZE_TEMPERATURE)
                .await
            {
                Ok(resp) => {
                    parse_membership(&extract_json(resp.content.trim()), batch.len(), open.is_some())
                }
                Err(e) => {
                    tracing::warn!(error = %e, "membership pass failed; absorbing batch");
                    MembershipDecision::absorb_all(batch.len())
                }
            };

            // Persist this batch's outcome atomically. The transaction is
            // scoped to a block: its binding must be dead before the next
            // await or the spawned future stops being Send.
            let k = decision.continue_through;
            let joining = &batch[..k];
            let session_id = {
            let tx = conn.unchecked_transaction().map_err(ApiError::internal)?;
            let session_id = match open {
                Some(ref s) => {
                    if k > 0 && let Some(ref label) = decision.label {
                        db::update_open_label(&tx, s.id, label).map_err(ApiError::internal)?;
                    }
                    s.id
                }
                None => {
                    let label = decision.label.as_deref().unwrap_or("Activity");
                    let lo = joining.first().map(|e| e.ts).unwrap_or(now);
                    let hi = joining.last().map(|e| e.ts).unwrap_or(now);
                    db::create_session(
                        &tx,
                        &range_key,
                        label,
                        lo,
                        hi,
                        decision.distraction,
                        "llm",
                        true,
                    )
                    .map_err(ApiError::internal)?
                }
            };
            for e in joining {
                db::assign_event_session(&tx, e.id, Some(session_id))
                    .map_err(ApiError::internal)?;
            }
            // Descriptions cover the whole batch; events past k keep theirs
            // for when the next iteration assigns them.
            for (i, desc) in &decision.descriptions {
                if let Some(e) = batch.get(i - 1) {
                    db::update_event_llm_desc(&tx, e.id, desc).map_err(ApiError::internal)?;
                }
            }
            tx.commit().map_err(ApiError::internal)?;
            session_id
            };

            open = db::get_session(&conn, session_id).map_err(ApiError::internal)?;

            if k < batch.len() {
                // The activity ended inside this batch: solidify, and let the
                // next iteration open a new session for the remainder.
                if let Some(ref s) = open {
                    let members = db::query_events_by_session(&conn, s.id).unwrap_or_default();
                    let (label, distraction) =
                        solidify_label(state.llm.as_ref(), &s.label, &members).await;
                    finish_close(&conn, s, if s.pinned { None } else { label }, distraction);
                }
                open = None;
            }
            // k == 0 only happens when an open session rejected the whole
            // batch; it was just solidified, so re-running the same batch
            // with no open session is guaranteed to absorb at least one
            // event (parse_membership forces k >= 1 then).
            idx += k;
        }
    }

    // Past days never keep an open session — only today is still happening.
    if range_key != day_range_key(now)
        && let Some(ref s) = open
    {
        let members = db::query_events_by_session(&conn, s.id).unwrap_or_default();
        let (label, distraction) = solidify_label(state.llm.as_ref(), &s.label, &members).await;
        finish_close(&conn, s, if s.pinned { None } else { label }, distraction);
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
    fn membership_lenient_parse() {
        let d = parse_membership(
            r#"{"continue_through":"3","label":" terminal — working on companion cube ","distraction":false,"descriptions":{"1":"editing rust code","9":"out of range","2":""}}"#,
            5,
            true,
        );
        assert_eq!(d.continue_through, 3);
        assert_eq!(d.label.as_deref(), Some("terminal — working on companion cube"));
        let mut descs = d.descriptions.clone();
        descs.sort();
        assert_eq!(descs, vec![(1, "editing rust code".to_string())]);
    }

    #[test]
    fn membership_clamps_and_forces_progress() {
        // over-long k clamps to batch size
        assert_eq!(parse_membership(r#"{"continue_through":99}"#, 4, true).continue_through, 4);
        // an open session may reject everything
        assert_eq!(parse_membership(r#"{"continue_through":0}"#, 4, true).continue_through, 0);
        // ...but a fresh activity must absorb at least its first event
        assert_eq!(parse_membership(r#"{"continue_through":0}"#, 4, false).continue_through, 1);
        // garbage absorbs everything rather than churning
        let d = parse_membership("not json", 4, true);
        assert_eq!(d.continue_through, 4);
        assert!(d.label.is_none());
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
}
