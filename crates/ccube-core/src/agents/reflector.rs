// Reflector agent — Phase 7 implementation.
//
// Consolidates a grown patterns.md into fewer, more abstract rules via a
// full-file rewrite. Runs weekly (Sunday 3am) or when patterns.md > 1600 chars.
// Uses a stricter eval gate than the curator: regressions == 0 AND change_ratio < 15%.

use crate::briefing::ReflectorOutput;
use crate::db::{self, CorrectionRow};
use crate::eval::{self, ReflectorEvalOutcome};
use crate::llm::{LlmBackend, LlmError};
use crate::memory;
use std::path::Path;

/// Prompt template version, logged with every reflector run.
pub const PROMPT_VERSION: &str = "reflector.v1";

/// GBNF grammar that constrains llama.cpp to produce valid ReflectorOutput JSON.
pub const REFLECTOR_GRAMMAR: &str = r#"
root ::= "{" ws
  "\"new_patterns_md\"" ws ":" ws string "," ws
  "\"rationale\"" ws ":" ws string
  ws "}"

string ::= "\"" chars "\""
chars ::= "" | char chars
char ::= [^"\\] | "\\" escape
escape ::= "\"" | "\\" | "/" | "b" | "f" | "n" | "r" | "t"

ws ::= | " " | "\n" | "\r" | "\t"
"#;

/// The JSON schema description embedded in the prompt.
const SCHEMA_DESC: &str = r#"{
  "new_patterns_md": "full text of the rewritten patterns.md",
  "rationale": "explanation of consolidation decisions"
}"#;

/// Errors specific to the reflector agent.
#[derive(Debug, thiserror::Error)]
pub enum ReflectorError {
    #[error("LLM unavailable: {0}")]
    LlmUnavailable(String),
    #[error("failed to parse reflector response: {0}")]
    ParseFailed(String),
}

/// Result of a full reflector run (orchestrator output).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ReflectorRunResult {
    pub patterns_before: String,
    pub patterns_after: String,
    pub rationale: String,
    pub eval_result: Option<eval::EvalResult>,
    pub eval_outcome: Option<ReflectorEvalOutcome>,
    pub committed: bool,
    pub pending: bool,
    pub dry_run: bool,
    pub chars_before: usize,
    pub chars_after: usize,
    pub retained_corrections_count: usize,
}

/// Render the reflector prompt by substituting placeholders in the template.
///
/// Uses a single-pass replacement approach (same as detector/curator) so that
/// user-provided content cannot collide with placeholder names.
pub fn render_prompt(profile: &str, patterns: &str, retained_corrections: &str) -> String {
    let template = include_str!("../prompts/reflector.v1.md");

    let replacements: &[(&str, &str)] = &[
        ("{profile}", profile),
        ("{patterns}", patterns),
        ("{retained_corrections}", retained_corrections),
        ("{schema}", SCHEMA_DESC),
    ];

    let mut result = String::with_capacity(template.len());
    let mut i = 0;
    while i < template.len() {
        if template.as_bytes()[i] == b'{' {
            let remaining = &template[i..];
            let mut matched = false;
            for &(placeholder, value) in replacements {
                if remaining.starts_with(placeholder) {
                    result.push_str(value);
                    i += placeholder.len();
                    matched = true;
                    break;
                }
            }
            if !matched {
                result.push('{');
                i += 1;
            }
        } else {
            let ch = &template[i..];
            let c = ch.chars().next().unwrap();
            result.push(c);
            i += c.len_utf8();
        }
    }

    result
}

/// Format retained corrections for the reflector prompt.
///
/// Uses a simplified format (no context fencing needed — the reflector only
/// needs to know what the user said, not the full briefing context).
pub fn format_retained_corrections(corrections: &[CorrectionRow]) -> String {
    if corrections.is_empty() {
        return "(none — no retained corrections in last 30 days)".to_string();
    }

    corrections
        .iter()
        .map(|c| {
            format!(
                "- Correction #{}: Detector said \"{}\", user said \"{}\"",
                c.id, c.original_decision, c.user_verdict
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Run the reflector LLM call: render prompt, call LLM, parse response.
///
/// Unlike the detector, the reflector does NOT silently fall back on error — it
/// returns `ReflectorError` so the caller can decide whether to retry.
pub async fn run(
    profile: &str,
    patterns: &str,
    retained_corrections: &str,
    llm: &dyn LlmBackend,
) -> Result<ReflectorOutput, ReflectorError> {
    let prompt = render_prompt(profile, patterns, retained_corrections);

    match llm.complete(&prompt, REFLECTOR_GRAMMAR, 2048, 0.4).await {
        // Via Value first so duplicate keys collapse last-wins (Ollama
        // ignores the grammar; see curator.rs for the same pattern).
        Ok(resp) => serde_json::from_str::<serde_json::Value>(&resp.content)
            .and_then(serde_json::from_value::<ReflectorOutput>)
            .map_err(|e| ReflectorError::ParseFailed(format!("{e}: {}", resp.content))),
        Err(LlmError::Unreachable(msg)) => Err(ReflectorError::LlmUnavailable(msg)),
        Err(LlmError::BadResponse(msg)) => Err(ReflectorError::ParseFailed(msg)),
    }
}

/// Full reflector orchestration: consolidate patterns, eval gate, write or queue.
///
/// Steps:
/// 1. Load retained corrections (last 30 days)
/// 2. Format corrections for prompt
/// 3. Run reflector LLM to get full-file rewrite
/// 4. If dry_run or no changes: return early
/// 5. Run eval replay against the proposed rewrite
/// 6. Based on outcome:
///    - Pass → commit to patterns.md
///    - Borderline → save as patterns.md.pending
///    - Fail → discard
/// 7. Log eval run to eval_runs.sqlite
#[allow(clippy::too_many_arguments)]
pub async fn run_reflector(
    data_dir: &Path,
    memory_dir: &Path,
    profile: &str,
    current_patterns: &str,
    llm: &dyn LlmBackend,
    eval_llm: &dyn LlmBackend,
    dry_run: bool,
) -> anyhow::Result<ReflectorRunResult> {
    let chars_before = current_patterns.len();

    // 1. Load retained corrections from the last 30 days
    let corr_conn = db::open_corrections_db(data_dir)?;
    let thirty_days_ms = 30_i64 * 24 * 60 * 60 * 1000;
    let since_ts = chrono::Utc::now().timestamp_millis() - thirty_days_ms;
    let retained = db::list_retained_corrections(&corr_conn, since_ts, 200)?;
    let retained_count = retained.len();

    // 2. Format corrections for prompt
    let formatted = format_retained_corrections(&retained);

    // 3. Run reflector LLM
    let output = run(profile, current_patterns, &formatted, llm).await?;
    let patterns_after = output.new_patterns_md.clone();
    let rationale = output.rationale.clone();
    let chars_after = patterns_after.len();

    // 4. Early return if dry_run
    if dry_run {
        return Ok(ReflectorRunResult {
            patterns_before: current_patterns.to_string(),
            patterns_after,
            rationale,
            eval_result: None,
            eval_outcome: None,
            committed: false,
            pending: false,
            dry_run: true,
            chars_before,
            chars_after,
            retained_corrections_count: retained_count,
        });
    }

    // 5. Early return if no actual changes
    if patterns_after.trim() == current_patterns.trim() {
        return Ok(ReflectorRunResult {
            patterns_before: current_patterns.to_string(),
            patterns_after,
            rationale,
            eval_result: None,
            eval_outcome: None,
            committed: false,
            pending: false,
            dry_run: false,
            chars_before,
            chars_after,
            retained_corrections_count: retained_count,
        });
    }

    // 6. Eval gate
    let events_conn = db::open_events_db(data_dir)?;
    let fourteen_days_ms = 14_i64 * 24 * 60 * 60 * 1000;
    let eval_since_ts = chrono::Utc::now().timestamp_millis() - fourteen_days_ms;
    let decisions = db::list_decisions(&events_conn, eval_since_ts, 10000)?;
    let all_corrections = db::list_corrections(&corr_conn, 10000, false)?;

    let eval_result = eval::replay(
        &decisions,
        &all_corrections,
        &patterns_after,
        profile,
        eval_llm,
    )
    .await?;

    let outcome = eval::reflector_passes(&eval_result);
    let mut committed = false;
    let mut pending = false;

    match outcome {
        ReflectorEvalOutcome::Pass => {
            memory::atomic_write_with_history(memory_dir, "patterns.md", &patterns_after, 30)?;
            committed = true;
        }
        ReflectorEvalOutcome::Borderline => {
            let pending_path = memory_dir.join("patterns.md.pending");
            std::fs::write(&pending_path, &patterns_after)?;
            pending = true;
            tracing::info!(
                change_ratio = format!(
                    "{:.1}%",
                    eval_result.decisions_changed as f64 / eval_result.events_replayed as f64
                        * 100.0
                ),
                "reflector: borderline result saved to patterns.md.pending"
            );
        }
        ReflectorEvalOutcome::Fail => {
            tracing::warn!(
                regressions = eval_result.regressions,
                "reflector: eval failed, discarding rewrite"
            );
        }
    }

    // 7. Log eval run
    let eval_conn = db::open_eval_runs_db(data_dir)?;
    let ts = chrono::Utc::now().timestamp_millis();
    if let Err(e) = db::insert_eval_run(
        &eval_conn,
        ts,
        "reflector",
        current_patterns,
        &patterns_after,
        eval_result.events_replayed as i64,
        eval_result.decisions_changed as i64,
        eval_result.regressions as i64,
        eval_result.passed,
        Some(&eval_result.rationale),
    ) {
        tracing::warn!(error = %e, "reflector: failed to log eval run");
    }

    Ok(ReflectorRunResult {
        patterns_before: current_patterns.to_string(),
        patterns_after,
        rationale,
        eval_result: Some(eval_result),
        eval_outcome: Some(outcome),
        committed,
        pending,
        dry_run: false,
        chars_before,
        chars_after,
        retained_corrections_count: retained_count,
    })
}

// ---------------------------------------------------------------------------
// Pending file management
// ---------------------------------------------------------------------------

/// Check whether a pending reflector proposal exists.
pub fn has_pending(memory_dir: &Path) -> bool {
    memory_dir.join("patterns.md.pending").exists()
}

/// Read the pending reflector proposal, if any.
pub fn read_pending(memory_dir: &Path) -> anyhow::Result<Option<String>> {
    let path = memory_dir.join("patterns.md.pending");
    if path.exists() {
        Ok(Some(std::fs::read_to_string(&path)?))
    } else {
        Ok(None)
    }
}

/// Accept the pending reflector proposal: commit to patterns.md, delete .pending.
pub fn accept_pending(memory_dir: &Path) -> anyhow::Result<()> {
    let path = memory_dir.join("patterns.md.pending");
    if !path.exists() {
        anyhow::bail!("no pending reflector proposal to accept");
    }
    let content = std::fs::read_to_string(&path)?;
    memory::atomic_write_with_history(memory_dir, "patterns.md", &content, 30)?;
    std::fs::remove_file(&path)?;
    Ok(())
}

/// Reject the pending reflector proposal: delete .pending.
pub fn reject_pending(memory_dir: &Path) -> anyhow::Result<()> {
    let path = memory_dir.join("patterns.md.pending");
    if !path.exists() {
        anyhow::bail!("no pending reflector proposal to reject");
    }
    std::fs::remove_file(&path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::briefing::{ActivitySnapshot, Briefing, FocusMode};
    use crate::llm::{LlmError, LlmResponse};
    use async_trait::async_trait;

    // ------------------------------------------------------------------
    // JSON/grammar tests
    // ------------------------------------------------------------------

    #[test]
    fn test_reflector_output_parses() {
        let json = r#"{
            "new_patterns_md": "§ coding in IDE is on-task\n§ social media during work is drift",
            "rationale": "merged 5 overlapping rules into 2 abstract principles"
        }"#;
        let output: ReflectorOutput = serde_json::from_str(json).unwrap();
        assert!(output.new_patterns_md.contains("coding in IDE"));
        assert!(output.rationale.contains("merged"));
    }

    // ------------------------------------------------------------------
    // Prompt rendering tests
    // ------------------------------------------------------------------

    #[test]
    fn test_render_prompt_no_placeholders() {
        let prompt = render_prompt("test profile", "test patterns", "test corrections");
        assert!(!prompt.contains("{profile}"));
        assert!(!prompt.contains("{patterns}"));
        assert!(!prompt.contains("{retained_corrections}"));
        assert!(!prompt.contains("{schema}"));
        assert!(prompt.contains("test profile"));
        assert!(prompt.contains("test patterns"));
        assert!(prompt.contains("test corrections"));
    }

    #[test]
    fn test_render_prompt_injection_safe() {
        let prompt = render_prompt(
            "profile with {patterns} in it",
            "REAL_PATTERNS",
            "corrections",
        );
        assert!(prompt.contains("{patterns}"));
        assert!(prompt.contains("REAL_PATTERNS"));
    }

    // ------------------------------------------------------------------
    // format_retained_corrections tests
    // ------------------------------------------------------------------

    #[test]
    fn test_format_retained_corrections_empty() {
        let result = format_retained_corrections(&[]);
        assert!(result.contains("none"));
    }

    #[test]
    fn test_format_retained_corrections_formats() {
        let corrections = vec![
            CorrectionRow {
                id: 1,
                ts: 1000,
                decision_id: 10,
                original_decision: "Nudge".to_string(),
                user_verdict: "was researching".to_string(),
                ctx_snapshot: "{}".to_string(),
                patterns_hash: "h1".to_string(),
                status: "retained".to_string(),
            },
            CorrectionRow {
                id: 2,
                ts: 2000,
                decision_id: 20,
                original_decision: "Silent".to_string(),
                user_verdict: "should have nudged".to_string(),
                ctx_snapshot: "{}".to_string(),
                patterns_hash: "h2".to_string(),
                status: "retained".to_string(),
            },
        ];

        let result = format_retained_corrections(&corrections);
        assert!(result.contains("Correction #1"));
        assert!(result.contains("Correction #2"));
        assert!(result.contains("was researching"));
        assert!(result.contains("should have nudged"));
        assert!(result.contains("Nudge"));
        assert!(result.contains("Silent"));
    }

    // ------------------------------------------------------------------
    // Mock LLMs
    // ------------------------------------------------------------------

    struct MockReflectorLlm {
        response: String,
    }

    #[async_trait]
    impl LlmBackend for MockReflectorLlm {
        async fn complete(
            &self,
            _prompt: &str,
            _grammar: &str,
            _n_predict: u32,
            _temperature: f32,
        ) -> Result<LlmResponse, LlmError> {
            Ok(LlmResponse {
                content: self.response.clone(),
                model: Some("test".to_string()),
            })
        }
    }

    struct FailingLlm;

    #[async_trait]
    impl LlmBackend for FailingLlm {
        async fn complete(
            &self,
            _prompt: &str,
            _grammar: &str,
            _n_predict: u32,
            _temperature: f32,
        ) -> Result<LlmResponse, LlmError> {
            Err(LlmError::Unreachable("mock down".into()))
        }
    }

    /// Mock LLM that returns "silent" detector output (for eval replay).
    struct AlwaysSilentDetectorLlm;

    #[async_trait]
    impl LlmBackend for AlwaysSilentDetectorLlm {
        async fn complete(
            &self,
            _prompt: &str,
            _grammar: &str,
            _n_predict: u32,
            _temperature: f32,
        ) -> Result<LlmResponse, LlmError> {
            Ok(LlmResponse {
                content: r#"{"decision":"silent","reasoning":"test","nudge_style":null,"nudge_message":null,"vault_category":null,"patterns_cited":[]}"#.to_string(),
                model: Some("test".to_string()),
            })
        }
    }

    // ------------------------------------------------------------------
    // run() tests
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_run_happy_path() {
        let json = r#"{"new_patterns_md":"§ coding is on-task\n§ social media is drift","rationale":"consolidated"}"#;
        let llm = MockReflectorLlm {
            response: json.to_string(),
        };
        let output = run("profile", "patterns", "corrections", &llm)
            .await
            .unwrap();
        assert!(output.new_patterns_md.contains("coding is on-task"));
        assert_eq!(output.rationale, "consolidated");
    }

    #[tokio::test]
    async fn test_run_llm_unavailable() {
        let llm = FailingLlm;
        let err = run("profile", "patterns", "corrections", &llm)
            .await
            .unwrap_err();
        assert!(matches!(err, ReflectorError::LlmUnavailable(_)));
    }

    #[tokio::test]
    async fn test_run_parse_failure() {
        let llm = MockReflectorLlm {
            response: "not valid json".to_string(),
        };
        let err = run("profile", "patterns", "corrections", &llm)
            .await
            .unwrap_err();
        assert!(matches!(err, ReflectorError::ParseFailed(_)));
    }

    // ------------------------------------------------------------------
    // run_reflector() orchestrator tests
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_run_reflector_dry_run() {
        let dir = tempfile::TempDir::new().unwrap();
        let memory_dir = dir.path().join("memory");
        std::fs::create_dir_all(&memory_dir).unwrap();
        db::init_databases(dir.path()).unwrap();

        let json = r#"{"new_patterns_md":"§ consolidated rule","rationale":"merged"}"#;
        let llm = MockReflectorLlm {
            response: json.to_string(),
        };
        let eval_llm = FailingLlm; // shouldn't be called in dry_run

        let result = run_reflector(
            dir.path(),
            &memory_dir,
            "profile",
            "§ old rule 1\n§ old rule 2",
            &llm,
            &eval_llm,
            true,
        )
        .await
        .unwrap();

        assert!(result.dry_run);
        assert!(!result.committed);
        assert!(!result.pending);
        assert!(result.eval_result.is_none());
        assert_eq!(result.patterns_after, "§ consolidated rule");
        // patterns.md should NOT have been written
        assert!(!memory_dir.join("patterns.md").exists());
    }

    fn make_test_briefing_json(patterns: &str) -> String {
        let b = Briefing {
            ts: 1000,
            active_mode: Some(FocusMode::Coding),
            right_now: ActivitySnapshot {
                app: "chrome.exe".to_string(),
                title: Some("Twitter".to_string()),
                url: None,
                duration_ms: 30000,
            },
            just_before: None,
            past_hour: vec![],
            calendar_hint: None,
            vault_today: vec![],
            profile_snippet: "test profile".to_string(),
            patterns_snippet: patterns.to_string(),
            patterns_hash: memory::patterns_hash(patterns),
        };
        serde_json::to_string(&b).unwrap()
    }

    #[tokio::test]
    async fn test_run_reflector_full_pass() {
        let dir = tempfile::TempDir::new().unwrap();
        let memory_dir = dir.path().join("memory");
        std::fs::create_dir_all(&memory_dir).unwrap();
        db::init_databases(dir.path()).unwrap();

        // Write initial bloated patterns
        let initial = "§ social media is always drift\n§ twitter is drift\n§ facebook is drift";
        memory::atomic_write_with_history(&memory_dir, "patterns.md", initial, 30).unwrap();
        let patterns_h = memory::patterns_hash(initial);

        // Insert a decision with valid briefing
        let briefing_json = make_test_briefing_json(initial);
        let events_conn = db::open_events_db(dir.path()).unwrap();
        db::insert_decision(
            &events_conn,
            chrono::Utc::now().timestamp_millis(),
            "heartbeat",
            "Nudge",
            "browsing twitter",
            Some("Gentle"),
            Some("hey"),
            &briefing_json,
            &patterns_h,
            "detector.v1",
            100,
        )
        .unwrap();
        drop(events_conn);

        // Reflector LLM proposes consolidated patterns
        let reflector_json = r#"{"new_patterns_md":"§ social media (twitter, facebook, etc.) is drift","rationale":"merged three social media rules into one"}"#;
        let reflector_llm = MockReflectorLlm {
            response: reflector_json.to_string(),
        };

        // Eval LLM returns "silent" — original was "Nudge", so decision changed.
        // 1 out of 1 = 100% change → that's >15%, so this would be Borderline.
        // But with only 1 decision, let's test with a more appropriate setup.
        // Actually let's just test with the AlwaysSilent mock and accept the outcome.
        let eval_llm = AlwaysSilentDetectorLlm;

        let result = run_reflector(
            dir.path(),
            &memory_dir,
            "test profile",
            initial,
            &reflector_llm,
            &eval_llm,
            false,
        )
        .await
        .unwrap();

        assert!(!result.dry_run);
        assert!(result.eval_result.is_some());
        let eval = result.eval_result.as_ref().unwrap();
        assert_eq!(eval.regressions, 0);
        // With 1 decision and 1 change = 100% → Borderline
        assert_eq!(result.eval_outcome, Some(ReflectorEvalOutcome::Borderline));
        assert!(!result.committed);
        assert!(result.pending);
        // .pending file should exist
        assert!(memory_dir.join("patterns.md.pending").exists());
    }

    #[tokio::test]
    async fn test_run_reflector_no_changes() {
        let dir = tempfile::TempDir::new().unwrap();
        let memory_dir = dir.path().join("memory");
        std::fs::create_dir_all(&memory_dir).unwrap();
        db::init_databases(dir.path()).unwrap();

        // LLM returns the same patterns (no change)
        let patterns = "§ existing rule";
        let json = r#"{"new_patterns_md":"§ existing rule","rationale":"nothing to change"}"#;
        let llm = MockReflectorLlm {
            response: json.to_string(),
        };
        let eval_llm = FailingLlm;

        let result = run_reflector(
            dir.path(),
            &memory_dir,
            "profile",
            patterns,
            &llm,
            &eval_llm,
            false,
        )
        .await
        .unwrap();

        assert!(!result.committed);
        assert!(!result.pending);
        assert!(result.eval_result.is_none());
    }

    // ------------------------------------------------------------------
    // Pending file management tests
    // ------------------------------------------------------------------

    #[test]
    fn test_has_pending() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(!has_pending(dir.path()));
        std::fs::write(dir.path().join("patterns.md.pending"), "proposed").unwrap();
        assert!(has_pending(dir.path()));
    }

    #[test]
    fn test_read_pending() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(read_pending(dir.path()).unwrap().is_none());
        std::fs::write(dir.path().join("patterns.md.pending"), "proposed").unwrap();
        assert_eq!(
            read_pending(dir.path()).unwrap().as_deref(),
            Some("proposed")
        );
    }

    #[test]
    fn test_accept_pending() {
        let dir = tempfile::TempDir::new().unwrap();
        // Write initial patterns
        memory::atomic_write_with_history(dir.path(), "patterns.md", "old", 30).unwrap();
        // Write pending
        std::fs::write(dir.path().join("patterns.md.pending"), "new patterns").unwrap();

        accept_pending(dir.path()).unwrap();

        assert_eq!(memory::read_patterns(dir.path()).unwrap(), "new patterns");
        assert!(!has_pending(dir.path()));
    }

    #[test]
    fn test_accept_pending_no_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let result = accept_pending(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_reject_pending() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("patterns.md.pending"), "proposed").unwrap();

        reject_pending(dir.path()).unwrap();
        assert!(!has_pending(dir.path()));
    }

    #[test]
    fn test_reject_pending_no_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let result = reject_pending(dir.path());
        assert!(result.is_err());
    }
}
