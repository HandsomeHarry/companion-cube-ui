// Curator agent — Phase 6 implementation.
//
// Processes pending corrections into patterns.md changes, gated by eval replay.
// Runs daily at a configurable time (default 5 AM local) or manually via CLI/HTTP.

use crate::briefing::CuratorOutput;
use crate::db::{self, CorrectionRow};
use crate::eval;
use crate::llm::{LlmBackend, LlmError};
use crate::memory;
use std::collections::HashMap;
use std::path::Path;

/// Prompt template version, logged with every curator run.
pub const PROMPT_VERSION: &str = "curator.v1";

/// GBNF grammar that constrains llama.cpp to produce valid CuratorOutput JSON.
pub const CURATOR_GRAMMAR: &str = r#"
root ::= "{" ws
  "\"correction_verdicts\"" ws ":" ws verdict-array "," ws
  "\"proposed_adds\"" ws ":" ws add-array "," ws
  "\"proposed_replaces\"" ws ":" ws replace-array "," ws
  "\"needs_reflection\"" ws ":" ws boolean "," ws
  "\"overall_rationale\"" ws ":" ws string
  ws "}"

verdict-array ::= "[]" | "[" ws verdict-obj ("," ws verdict-obj)* ws "]"
verdict-obj ::= "{" ws
  "\"correction_id\"" ws ":" ws int "," ws
  "\"verdict\"" ws ":" ws verdict-val "," ws
  "\"rationale\"" ws ":" ws string ws "}"

verdict-val ::= "\"retain\"" | "\"discard\"" | "\"defer\""

add-array ::= "[]" | "[" ws add-obj ("," ws add-obj)* ws "]"
add-obj ::= "{" ws
  "\"text\"" ws ":" ws string "," ws
  "\"supporting_correction_ids\"" ws ":" ws int-array ws "}"

replace-array ::= "[]" | "[" ws replace-obj ("," ws replace-obj)* ws "]"
replace-obj ::= "{" ws
  "\"old_text\"" ws ":" ws string "," ws
  "\"new_text\"" ws ":" ws string "," ws
  "\"rationale\"" ws ":" ws string ws "}"

boolean ::= "true" | "false"
int-array ::= "[]" | "[" ws int ("," ws int)* ws "]"
int ::= [0-9]+

string ::= "\"" chars "\""
chars ::= "" | char chars
char ::= [^"\\] | "\\" escape
escape ::= "\"" | "\\" | "/" | "b" | "f" | "n" | "r" | "t"

ws ::= | " " | "\n" | "\r" | "\t"
"#;

/// The JSON schema description embedded in the prompt.
const SCHEMA_DESC: &str = r#"{
  "correction_verdicts": [
    {"correction_id": 1, "verdict": "retain" | "discard" | "defer", "rationale": "one sentence"}
  ],
  "proposed_adds": [
    {"text": "pattern line under 120 chars", "supporting_correction_ids": [1, 2]}
  ],
  "proposed_replaces": [
    {"old_text": "exact old pattern line", "new_text": "new pattern line", "rationale": "why"}
  ],
  "needs_reflection": true | false,
  "overall_rationale": "summary sentence"
}"#;

/// Errors specific to the curator agent.
#[derive(Debug, thiserror::Error)]
pub enum CuratorError {
    #[error("LLM unavailable: {0}")]
    LlmUnavailable(String),
    #[error("failed to parse curator response: {0}")]
    ParseFailed(String),
}

/// Result of a full curator run (orchestrator output).
#[derive(Debug, Clone, serde::Serialize)]
pub struct CuratorRunResult {
    pub corrections_processed: usize,
    pub output: CuratorOutput,
    pub candidate_patterns: String,
    pub eval_result: Option<eval::EvalResult>,
    pub committed: bool,
    pub dry_run: bool,
}

/// Render the curator prompt by substituting placeholders in the template.
///
/// Uses a single-pass replacement approach (same as detector) so that user-provided
/// content cannot collide with placeholder names.
pub fn render_prompt(profile: &str, current_patterns: &str, formatted_corrections: &str) -> String {
    let template = include_str!("../prompts/curator.v1.md");
    let patterns_char_count = current_patterns.len().to_string();

    let replacements: &[(&str, &str)] = &[
        ("{profile}", profile),
        ("{patterns_char_count}", &patterns_char_count),
        ("{patterns}", current_patterns),
        ("{corrections}", formatted_corrections),
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

/// Format correction blocks for the curator prompt, with context fencing.
///
/// Each correction sees the patterns as they existed at the time of the original
/// detector decision, not the current patterns. The `hash_cache` maps patterns_hash
/// values to their content.
pub fn format_corrections(
    corrections: &[CorrectionRow],
    hash_cache: &HashMap<String, String>,
) -> String {
    let mut blocks = Vec::with_capacity(corrections.len());

    for c in corrections {
        let patterns_at_time = hash_cache
            .get(&c.patterns_hash)
            .cloned()
            .unwrap_or_default();

        let patterns_display = if patterns_at_time.is_empty() {
            "(no patterns existed yet)".to_string()
        } else {
            patterns_at_time
        };

        // Truncate ctx_snapshot to ~500 chars for prompt budget (UTF-8 safe)
        let ctx_display = if c.ctx_snapshot.len() > 500 {
            let mut end = 500;
            while !c.ctx_snapshot.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}...", &c.ctx_snapshot[..end])
        } else {
            c.ctx_snapshot.clone()
        };

        blocks.push(format!(
            "--- Correction #{id} ---\n\
             Detector decided: {original}\n\
             User verdict: \"{verdict}\"\n\
             Patterns at decision time:\n\
             {patterns}\n\
             Context snapshot (briefing): {ctx}",
            id = c.id,
            original = c.original_decision,
            verdict = c.user_verdict,
            patterns = patterns_display,
            ctx = ctx_display,
        ));
    }

    blocks.join("\n\n")
}

/// Apply curator's proposed changes to the current patterns.
///
/// Pure function: applies `proposed_replaces` first (exact substring match),
/// then appends `proposed_adds` as new lines.
pub fn apply_changes(current_patterns: &str, output: &CuratorOutput) -> String {
    let mut result = current_patterns.to_string();

    for replace in &output.proposed_replaces {
        if result.contains(&replace.old_text) {
            result = result.replace(&replace.old_text, &replace.new_text);
        } else {
            tracing::warn!(
                old_text = %replace.old_text,
                "curator: proposed replace target not found in patterns, skipping"
            );
        }
    }

    for add in &output.proposed_adds {
        if !add.text.is_empty() {
            if !result.is_empty() && !result.ends_with('\n') {
                result.push('\n');
            }
            result.push_str(&add.text);
            result.push('\n');
        }
    }

    result
}

/// Run the curator LLM call: render prompt, call LLM, parse response.
///
/// Unlike the detector, the curator does NOT silently fall back on error — it
/// returns `CuratorError` so the caller can decide whether to retry.
pub async fn run(
    profile: &str,
    current_patterns: &str,
    formatted_corrections: &str,
    llm: &dyn LlmBackend,
) -> Result<CuratorOutput, CuratorError> {
    let prompt = render_prompt(profile, current_patterns, formatted_corrections);

    match llm.complete(&prompt, CURATOR_GRAMMAR, 1024, 0.4).await {
        // Parse via Value first: duplicate keys (which sloppy local models
        // emit, and Ollama's ignored grammar can't prevent) then collapse to
        // last-wins instead of failing the whole run.
        Ok(resp) => serde_json::from_str::<serde_json::Value>(&resp.content)
            .and_then(serde_json::from_value::<CuratorOutput>)
            .map_err(|e| CuratorError::ParseFailed(format!("{e}: {}", resp.content))),
        Err(LlmError::Unreachable(msg)) => Err(CuratorError::LlmUnavailable(msg)),
        Err(LlmError::BadResponse(msg)) => Err(CuratorError::ParseFailed(msg)),
    }
}

/// Full curator orchestration: process corrections, propose changes, eval gate, write.
///
/// Steps:
/// 1. Load pending corrections (return early if none)
/// 2. Build patterns hash cache for context fencing
/// 3. Run curator LLM to get verdicts + proposals
/// 4. Apply proposed changes to get candidate patterns
/// 5. Run eval replay (unless dry_run or no changes)
/// 6. If eval passes: commit changes to patterns.md, update correction statuses
/// 7. Log eval run to eval_runs.sqlite
#[allow(clippy::too_many_arguments)]
pub async fn run_curator(
    data_dir: &Path,
    memory_dir: &Path,
    profile: &str,
    current_patterns: &str,
    llm: &dyn LlmBackend,
    eval_llm: &dyn LlmBackend,
    dry_run: bool,
) -> anyhow::Result<CuratorRunResult> {
    // 1. Load pending corrections
    let corr_conn = db::open_corrections_db(data_dir)?;
    let corrections = db::list_corrections(&corr_conn, 500, true)?;

    if corrections.is_empty() {
        return Ok(CuratorRunResult {
            corrections_processed: 0,
            output: CuratorOutput {
                correction_verdicts: vec![],
                proposed_adds: vec![],
                proposed_replaces: vec![],
                needs_reflection: false,
                overall_rationale: "no pending corrections".to_string(),
            },
            candidate_patterns: current_patterns.to_string(),
            eval_result: None,
            committed: false,
            dry_run,
        });
    }

    // 2. Build context fencing cache
    let hash_cache = memory::build_patterns_hash_cache(memory_dir)?;

    // 3. Format corrections with fenced patterns
    let formatted = format_corrections(&corrections, &hash_cache);

    // 4. Run curator LLM
    let output = run(profile, current_patterns, &formatted, llm).await?;

    // 5. Apply proposed changes
    let candidate_patterns = apply_changes(current_patterns, &output);
    let has_changes = candidate_patterns.trim() != current_patterns.trim();

    // 6. Eval gate (skip for dry_run or no actual changes)
    let mut committed = false;
    let eval_result = if dry_run || !has_changes {
        None
    } else {
        // Load decisions for eval replay (last 14 days)
        let events_conn = db::open_events_db(data_dir)?;
        let fourteen_days_ms = 14_i64 * 24 * 60 * 60 * 1000;
        let since_ts = chrono::Utc::now().timestamp_millis() - fourteen_days_ms;
        let decisions = db::list_decisions(&events_conn, since_ts, 10000)?;
        let all_corrections = db::list_corrections(&corr_conn, 10000, false)?;

        let result = eval::replay(
            &decisions,
            &all_corrections,
            &candidate_patterns,
            profile,
            eval_llm,
        )
        .await?;

        if eval::curator_passes(&result) {
            // Eval passed — commit changes
            memory::atomic_write_with_history(
                memory_dir,
                "patterns.md",
                &candidate_patterns,
                30,
            )?;

            // Update correction statuses based on verdicts
            for verdict in &output.correction_verdicts {
                let status = match verdict.verdict.as_str() {
                    "retain" => "retained",
                    "discard" => "discarded",
                    "defer" => "deferred",
                    _ => "pending",
                };
                if let Err(e) =
                    db::update_correction_status(&corr_conn, verdict.correction_id, status)
                {
                    tracing::warn!(
                        correction_id = verdict.correction_id,
                        error = %e,
                        "curator: failed to update correction status"
                    );
                }
            }

            committed = true;
        }

        // Log eval run to eval_runs.sqlite
        let eval_conn = db::open_eval_runs_db(data_dir)?;
        let ts = chrono::Utc::now().timestamp_millis();
        if let Err(e) = db::insert_eval_run(
            &eval_conn,
            ts,
            "curator",
            current_patterns,
            &candidate_patterns,
            result.events_replayed as i64,
            result.decisions_changed as i64,
            result.regressions as i64,
            result.passed,
            Some(&result.rationale),
        ) {
            tracing::warn!(error = %e, "curator: failed to log eval run");
        }

        Some(result)
    };

    Ok(CuratorRunResult {
        corrections_processed: corrections.len(),
        output,
        candidate_patterns,
        eval_result,
        committed,
        dry_run,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::briefing::{ActivitySnapshot, FocusMode, PatternAdd, PatternReplace};
    use crate::llm::{LlmError, LlmResponse};
    use async_trait::async_trait;

    // ------------------------------------------------------------------
    // JSON/grammar tests
    // ------------------------------------------------------------------

    #[test]
    fn test_curator_output_parses() {
        let json = r#"{
            "correction_verdicts": [
                {"correction_id": 1, "verdict": "retain", "rationale": "valid pattern"},
                {"correction_id": 2, "verdict": "discard", "rationale": "one-off"}
            ],
            "proposed_adds": [
                {"text": "HN after 2pm in Coding mode is drift", "supporting_correction_ids": [1]}
            ],
            "proposed_replaces": [],
            "needs_reflection": false,
            "overall_rationale": "one pattern retained"
        }"#;
        let output: CuratorOutput = serde_json::from_str(json).unwrap();
        assert_eq!(output.correction_verdicts.len(), 2);
        assert_eq!(output.correction_verdicts[0].verdict, "retain");
        assert_eq!(output.correction_verdicts[1].verdict, "discard");
        assert_eq!(output.proposed_adds.len(), 1);
        assert!(output.proposed_replaces.is_empty());
        assert!(!output.needs_reflection);
    }

    #[test]
    fn test_curator_output_with_replaces() {
        let json = r#"{
            "correction_verdicts": [{"correction_id": 1, "verdict": "retain", "rationale": "ok"}],
            "proposed_adds": [],
            "proposed_replaces": [
                {"old_text": "social media is always drift", "new_text": "social media is drift except breaks", "rationale": "too aggressive"}
            ],
            "needs_reflection": true,
            "overall_rationale": "adjusted existing rule"
        }"#;
        let output: CuratorOutput = serde_json::from_str(json).unwrap();
        assert_eq!(output.proposed_replaces.len(), 1);
        assert_eq!(
            output.proposed_replaces[0].old_text,
            "social media is always drift"
        );
        assert!(output.needs_reflection);
    }

    // ------------------------------------------------------------------
    // Prompt rendering tests
    // ------------------------------------------------------------------

    #[test]
    fn test_render_prompt_no_placeholders() {
        let prompt = render_prompt("test profile", "test patterns", "test corrections");
        assert!(!prompt.contains("{profile}"));
        assert!(!prompt.contains("{patterns}"));
        assert!(!prompt.contains("{patterns_char_count}"));
        assert!(!prompt.contains("{corrections}"));
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

    #[test]
    fn test_render_prompt_char_count() {
        let patterns = "0123456789"; // 10 chars
        let prompt = render_prompt("p", patterns, "c");
        assert!(prompt.contains("10"));
    }

    // ------------------------------------------------------------------
    // Context fencing: format_corrections
    // ------------------------------------------------------------------

    #[test]
    fn test_format_corrections_with_fenced_patterns() {
        let mut cache = HashMap::new();
        cache.insert("hash_v1".to_string(), "§ old pattern".to_string());
        cache.insert("hash_v2".to_string(), "§ new pattern".to_string());

        let corrections = vec![
            CorrectionRow {
                id: 1,
                ts: 1000,
                decision_id: 10,
                original_decision: "Nudge".to_string(),
                user_verdict: "was fine".to_string(),
                ctx_snapshot: r#"{"ts":1000}"#.to_string(),
                patterns_hash: "hash_v1".to_string(),
                status: "pending".to_string(),
            },
            CorrectionRow {
                id: 2,
                ts: 2000,
                decision_id: 20,
                original_decision: "Silent".to_string(),
                user_verdict: "should nudge".to_string(),
                ctx_snapshot: r#"{"ts":2000}"#.to_string(),
                patterns_hash: "hash_v2".to_string(),
                status: "pending".to_string(),
            },
        ];

        let result = format_corrections(&corrections, &cache);
        assert!(result.contains("Correction #1"));
        assert!(result.contains("Correction #2"));
        assert!(result.contains("§ old pattern"));
        assert!(result.contains("§ new pattern"));
        assert!(result.contains("was fine"));
        assert!(result.contains("should nudge"));
    }

    #[test]
    fn test_format_corrections_missing_hash() {
        let cache = HashMap::new();
        let corrections = vec![CorrectionRow {
            id: 1,
            ts: 1000,
            decision_id: 10,
            original_decision: "Nudge".to_string(),
            user_verdict: "wrong".to_string(),
            ctx_snapshot: "{}".to_string(),
            patterns_hash: "unknown_hash".to_string(),
            status: "pending".to_string(),
        }];

        let result = format_corrections(&corrections, &cache);
        assert!(result.contains("(no patterns existed yet)"));
    }

    #[test]
    fn test_format_corrections_truncates_long_ctx() {
        let cache = HashMap::new();
        let long_ctx = "x".repeat(1000);
        let corrections = vec![CorrectionRow {
            id: 1,
            ts: 1000,
            decision_id: 10,
            original_decision: "Nudge".to_string(),
            user_verdict: "wrong".to_string(),
            ctx_snapshot: long_ctx,
            patterns_hash: "h".to_string(),
            status: "pending".to_string(),
        }];

        let result = format_corrections(&corrections, &cache);
        // Should be truncated with "..."
        assert!(result.contains("..."));
        // The 1000-char ctx_snapshot should have been truncated to ~500
        // (surrounding text like "Context" and "existed" also contain 'x')
        assert!(result.len() < 1000);
    }

    // ------------------------------------------------------------------
    // apply_changes
    // ------------------------------------------------------------------

    #[test]
    fn test_apply_changes_adds() {
        let output = CuratorOutput {
            correction_verdicts: vec![],
            proposed_adds: vec![PatternAdd {
                text: "§ HN is drift".to_string(),
                supporting_correction_ids: vec![1],
            }],
            proposed_replaces: vec![],
            needs_reflection: false,
            overall_rationale: "test".to_string(),
        };

        let result = apply_changes("§ existing pattern", &output);
        assert!(result.contains("§ existing pattern"));
        assert!(result.contains("§ HN is drift"));
    }

    #[test]
    fn test_apply_changes_replaces() {
        let output = CuratorOutput {
            correction_verdicts: vec![],
            proposed_adds: vec![],
            proposed_replaces: vec![PatternReplace {
                old_text: "social media is always drift".to_string(),
                new_text: "social media is drift except during breaks".to_string(),
                rationale: "too aggressive".to_string(),
            }],
            needs_reflection: false,
            overall_rationale: "test".to_string(),
        };

        let result =
            apply_changes("§ social media is always drift\n§ coding is on-task", &output);
        assert!(result.contains("social media is drift except during breaks"));
        assert!(!result.contains("social media is always drift"));
        assert!(result.contains("§ coding is on-task"));
    }

    #[test]
    fn test_apply_changes_replace_not_found() {
        let output = CuratorOutput {
            correction_verdicts: vec![],
            proposed_adds: vec![],
            proposed_replaces: vec![PatternReplace {
                old_text: "nonexistent pattern".to_string(),
                new_text: "replacement".to_string(),
                rationale: "test".to_string(),
            }],
            needs_reflection: false,
            overall_rationale: "test".to_string(),
        };

        let result = apply_changes("§ existing pattern", &output);
        assert_eq!(result, "§ existing pattern");
    }

    #[test]
    fn test_apply_changes_adds_and_replaces() {
        let output = CuratorOutput {
            correction_verdicts: vec![],
            proposed_adds: vec![PatternAdd {
                text: "§ new rule".to_string(),
                supporting_correction_ids: vec![1],
            }],
            proposed_replaces: vec![PatternReplace {
                old_text: "§ old rule".to_string(),
                new_text: "§ updated rule".to_string(),
                rationale: "improved".to_string(),
            }],
            needs_reflection: false,
            overall_rationale: "test".to_string(),
        };

        let result = apply_changes("§ old rule\n§ keep this", &output);
        assert!(result.contains("§ updated rule"));
        assert!(result.contains("§ keep this"));
        assert!(result.contains("§ new rule"));
        assert!(!result.contains("§ old rule"));
    }

    #[test]
    fn test_apply_changes_empty_add_skipped() {
        let output = CuratorOutput {
            correction_verdicts: vec![],
            proposed_adds: vec![PatternAdd {
                text: String::new(),
                supporting_correction_ids: vec![],
            }],
            proposed_replaces: vec![],
            needs_reflection: false,
            overall_rationale: "test".to_string(),
        };

        let result = apply_changes("§ existing", &output);
        assert_eq!(result, "§ existing");
    }

    // ------------------------------------------------------------------
    // Mock LLMs
    // ------------------------------------------------------------------

    struct MockCuratorLlm {
        response: String,
    }

    #[async_trait]
    impl LlmBackend for MockCuratorLlm {
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
        let json = r#"{"correction_verdicts":[{"correction_id":1,"verdict":"retain","rationale":"valid"}],"proposed_adds":[{"text":"§ new pattern","supporting_correction_ids":[1]}],"proposed_replaces":[],"needs_reflection":false,"overall_rationale":"ok"}"#;
        let llm = MockCuratorLlm {
            response: json.to_string(),
        };
        let output = run("profile", "patterns", "corrections", &llm)
            .await
            .unwrap();
        assert_eq!(output.correction_verdicts.len(), 1);
        assert_eq!(output.proposed_adds.len(), 1);
        assert_eq!(output.overall_rationale, "ok");
    }

    #[tokio::test]
    async fn test_run_llm_unavailable() {
        let llm = FailingLlm;
        let err = run("profile", "patterns", "corrections", &llm)
            .await
            .unwrap_err();
        assert!(matches!(err, CuratorError::LlmUnavailable(_)));
    }

    #[tokio::test]
    async fn test_run_parse_failure() {
        let llm = MockCuratorLlm {
            response: "not valid json".to_string(),
        };
        let err = run("profile", "patterns", "corrections", &llm)
            .await
            .unwrap_err();
        assert!(matches!(err, CuratorError::ParseFailed(_)));
    }

    // ------------------------------------------------------------------
    // run_curator() orchestrator tests
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_run_curator_no_corrections() {
        let dir = tempfile::TempDir::new().unwrap();
        let memory_dir = dir.path().join("memory");
        std::fs::create_dir_all(&memory_dir).unwrap();
        db::init_databases(dir.path()).unwrap();

        let llm = FailingLlm; // shouldn't be called
        let result = run_curator(dir.path(), &memory_dir, "profile", "patterns", &llm, &llm, false)
            .await
            .unwrap();

        assert_eq!(result.corrections_processed, 0);
        assert!(!result.committed);
        assert_eq!(result.output.overall_rationale, "no pending corrections");
    }

    #[tokio::test]
    async fn test_run_curator_dry_run() {
        let dir = tempfile::TempDir::new().unwrap();
        let memory_dir = dir.path().join("memory");
        std::fs::create_dir_all(&memory_dir).unwrap();
        db::init_databases(dir.path()).unwrap();

        // Insert a pending correction
        let corr_conn = db::open_corrections_db(dir.path()).unwrap();
        db::insert_correction(&corr_conn, 1, "Nudge", "was fine", "{}", "somehash").unwrap();
        drop(corr_conn);

        let json = r#"{"correction_verdicts":[{"correction_id":1,"verdict":"retain","rationale":"valid"}],"proposed_adds":[{"text":"§ new pattern","supporting_correction_ids":[1]}],"proposed_replaces":[],"needs_reflection":false,"overall_rationale":"ok"}"#;
        let curator_llm = MockCuratorLlm {
            response: json.to_string(),
        };
        let eval_llm = FailingLlm; // shouldn't be called in dry_run

        let result = run_curator(
            dir.path(),
            &memory_dir,
            "profile",
            "§ existing",
            &curator_llm,
            &eval_llm,
            true,
        )
        .await
        .unwrap();

        assert_eq!(result.corrections_processed, 1);
        assert!(!result.committed);
        assert!(result.dry_run);
        assert!(result.eval_result.is_none());
        // patterns.md should NOT have been written
        assert!(!memory_dir.join("patterns.md").exists());
    }

    fn make_test_briefing_json(patterns: &str) -> String {
        use crate::briefing::Briefing;
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
    async fn test_run_curator_full_with_eval_pass() {
        let dir = tempfile::TempDir::new().unwrap();
        let memory_dir = dir.path().join("memory");
        std::fs::create_dir_all(&memory_dir).unwrap();
        db::init_databases(dir.path()).unwrap();

        // Write initial patterns
        let initial_patterns = "§ social media is always drift";
        memory::atomic_write_with_history(&memory_dir, "patterns.md", initial_patterns, 30)
            .unwrap();
        let patterns_h = memory::patterns_hash(initial_patterns);

        // Insert a decision with valid briefing_json (original was "Nudge")
        let briefing_json = make_test_briefing_json(initial_patterns);
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

        // Insert a correction referencing that decision
        let corr_conn = db::open_corrections_db(dir.path()).unwrap();
        db::insert_correction(
            &corr_conn,
            1, // decision_id
            "Nudge",
            "was fine, I was researching",
            &briefing_json,
            &patterns_h,
        )
        .unwrap();
        drop(corr_conn);

        // Curator LLM proposes a new pattern
        let curator_json = r#"{"correction_verdicts":[{"correction_id":1,"verdict":"retain","rationale":"valid correction"}],"proposed_adds":[{"text":"§ twitter research during Coding is on-task","supporting_correction_ids":[1]}],"proposed_replaces":[],"needs_reflection":false,"overall_rationale":"added research exception"}"#;
        let curator_llm = MockCuratorLlm {
            response: curator_json.to_string(),
        };

        // Eval LLM returns "silent" for all replays.
        // Original decision was "Nudge", candidate returns "Silent" → decision changed.
        // The correction is on decision_id=1, original was "Nudge", candidate returns
        // "Silent" (different) → NOT a regression (the fix worked).
        let eval_llm = AlwaysSilentDetectorLlm;

        let result = run_curator(
            dir.path(),
            &memory_dir,
            "test profile",
            initial_patterns,
            &curator_llm,
            &eval_llm,
            false,
        )
        .await
        .unwrap();

        assert_eq!(result.corrections_processed, 1);
        assert!(result.committed);
        assert!(!result.dry_run);
        assert!(result.eval_result.is_some());
        let eval = result.eval_result.as_ref().unwrap();
        assert!(eval.passed);
        assert_eq!(eval.regressions, 0);
        assert!(eval.decisions_changed > 0);

        // patterns.md should have been updated
        let new_patterns = memory::read_patterns(&memory_dir).unwrap();
        assert!(new_patterns.contains("§ twitter research during Coding is on-task"));

        // Correction status should have been updated
        let corr_conn = db::open_corrections_db(dir.path()).unwrap();
        let c = db::get_correction(&corr_conn, 1).unwrap().unwrap();
        assert_eq!(c.status, "retained");

        // Eval run should have been logged
        let eval_conn = db::open_eval_runs_db(dir.path()).unwrap();
        let runs = db::list_eval_runs(&eval_conn, 10).unwrap();
        assert_eq!(runs.len(), 1);
        assert!(runs[0].passed);
    }
}
