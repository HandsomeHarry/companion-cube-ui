// Eval replay harness — Phase 6 implementation.
//
// Replays historical detector decisions against candidate patterns to check
// for regressions before committing curator/reflector changes to patterns.md.

use crate::agents::detector;
use crate::briefing::{Briefing, DetectorDecision};
use crate::db::{CorrectionRow, DecisionRow};
use crate::llm::LlmBackend;
use crate::memory;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Result of an eval replay run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResult {
    /// Total number of historical decisions replayed.
    pub events_replayed: usize,
    /// Decisions that differ from the original when using candidate patterns.
    pub decisions_changed: usize,
    /// Changed decisions that contradict a user correction (the critical metric).
    pub regressions: usize,
    /// Whether the eval passed its criteria.
    pub passed: bool,
    /// Human-readable summary.
    pub rationale: String,
    /// Wall time of the replay in milliseconds.
    pub duration_ms: u64,
}

/// Check whether an eval result meets the curator's pass criteria:
/// no regressions, and the candidate patterns actually changed some decisions.
pub fn curator_passes(result: &EvalResult) -> bool {
    result.regressions == 0 && result.decisions_changed > 0
}

/// Outcome of the reflector's stricter eval gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReflectorEvalOutcome {
    /// Clean pass: no regressions and change ratio under 15%.
    Pass,
    /// No regressions but too many decisions changed (>= 15%) — save as pending for review.
    Borderline,
    /// Regressions detected — reject the rewrite.
    Fail,
}

/// Check whether an eval result meets the reflector's stricter pass criteria.
///
/// The reflector should refine, not revolutionize. If it changes more than 15%
/// of decisions, the rewrite is too aggressive and goes to pending review.
pub fn reflector_passes(result: &EvalResult) -> ReflectorEvalOutcome {
    if result.regressions > 0 {
        return ReflectorEvalOutcome::Fail;
    }
    if result.events_replayed == 0 {
        return ReflectorEvalOutcome::Pass;
    }
    let change_ratio = result.decisions_changed as f64 / result.events_replayed as f64;
    if change_ratio < 0.15 {
        ReflectorEvalOutcome::Pass
    } else {
        ReflectorEvalOutcome::Borderline
    }
}

/// Replay historical detector decisions against candidate patterns.
///
/// Samples from the decision log at ~30-min intervals, always including
/// decisions that have associated corrections. For each sample, swaps in the
/// candidate patterns and re-runs the detector, then compares against the
/// original decision and any corrections.
pub async fn replay(
    decisions: &[DecisionRow],
    corrections: &[CorrectionRow],
    candidate_patterns: &str,
    profile: &str,
    llm: &dyn LlmBackend,
) -> anyhow::Result<EvalResult> {
    let start = std::time::Instant::now();

    if decisions.is_empty() {
        return Ok(EvalResult {
            events_replayed: 0,
            decisions_changed: 0,
            regressions: 0,
            passed: true,
            rationale: "no decisions to replay".to_string(),
            duration_ms: 0,
        });
    }

    // Build correction lookup: decision_id -> CorrectionRow
    let correction_map: HashMap<i64, &CorrectionRow> = corrections
        .iter()
        .map(|c| (c.decision_id, c))
        .collect();

    // Sample decisions: always include those with corrections, then fill with
    // wider-interval samples up to ~150 total.
    let samples = select_samples(decisions, &correction_map, 150);

    let candidate_hash = memory::patterns_hash(candidate_patterns);
    let mut decisions_changed: usize = 0;
    let mut regressions: usize = 0;
    let mut replayed: usize = 0;

    for decision in &samples {
        // Budget enforcement: if we've spent > 90s, stop
        if start.elapsed().as_secs() > 90 {
            tracing::warn!(
                replayed,
                total = samples.len(),
                "eval replay budget exceeded, stopping early"
            );
            break;
        }

        // Deserialize the stored briefing and swap in candidate patterns
        let briefing: Briefing = match serde_json::from_str(&decision.briefing_json) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(
                    decision_id = decision.id,
                    error = %e,
                    "eval: failed to deserialize briefing_json, skipping"
                );
                continue;
            }
        };

        let modified = Briefing {
            patterns_snippet: candidate_patterns.to_string(),
            patterns_hash: candidate_hash.clone(),
            profile_snippet: profile.to_string(),
            ..briefing
        };

        // Re-run the detector with candidate patterns
        let output = detector::run(&modified, llm).await;
        replayed += 1;

        // Compare: did the decision change?
        let original_decision = parse_decision(&decision.decision);
        let changed = output.decision != original_decision;
        if changed {
            decisions_changed += 1;
        }

        // Check for regression: if there's a correction for this decision and
        // the candidate decision still matches the original (the one the user
        // objected to), that's a regression. This catches both "no change" and
        // "changed to something then reverted" cases at correction points.
        if correction_map.contains_key(&decision.id)
            && output.decision == original_decision
        {
            // The user corrected this decision, but the candidate patterns
            // still produce the same wrong answer.
            regressions += 1;
        }
    }

    let duration_ms = start.elapsed().as_millis() as u64;
    let passed = regressions == 0 && decisions_changed > 0;

    let rationale = if replayed == 0 {
        "no decisions could be replayed".to_string()
    } else if regressions > 0 {
        format!(
            "{regressions} regression(s) in {replayed} replayed decisions ({decisions_changed} changed)"
        )
    } else if decisions_changed == 0 {
        format!("no decisions changed across {replayed} replays — candidate patterns may be cosmetic-only")
    } else {
        format!(
            "{decisions_changed} decision(s) changed, 0 regressions across {replayed} replays"
        )
    };

    Ok(EvalResult {
        events_replayed: replayed,
        decisions_changed,
        regressions,
        passed,
        rationale,
        duration_ms,
    })
}

/// Select a subset of decisions to replay. Always includes decisions that have
/// corrections. Fills remaining budget with wider-interval samples.
fn select_samples<'a>(
    decisions: &'a [DecisionRow],
    correction_map: &HashMap<i64, &CorrectionRow>,
    max_samples: usize,
) -> Vec<&'a DecisionRow> {
    // Phase 1: mandatory samples — decisions with corrections
    let mut mandatory: Vec<&DecisionRow> = decisions
        .iter()
        .filter(|d| correction_map.contains_key(&d.id))
        .collect();

    let remaining_budget = max_samples.saturating_sub(mandatory.len());

    if remaining_budget == 0 || decisions.len() <= mandatory.len() {
        return mandatory;
    }

    // Phase 2: fill with evenly-spaced samples from non-correction decisions
    let non_correction: Vec<&DecisionRow> = decisions
        .iter()
        .filter(|d| !correction_map.contains_key(&d.id))
        .collect();

    if non_correction.is_empty() {
        return mandatory;
    }

    let step = (non_correction.len() / remaining_budget).max(1);
    let fill: Vec<&DecisionRow> = non_correction
        .iter()
        .step_by(step)
        .take(remaining_budget)
        .copied()
        .collect();

    mandatory.extend(fill);
    // Sort by timestamp for consistent ordering
    mandatory.sort_by_key(|d| d.ts);
    mandatory
}

/// Parse a DetectorDecision from its Debug format string (e.g. "Nudge", "Silent", "Vault").
fn parse_decision(s: &str) -> DetectorDecision {
    match s.to_lowercase().as_str() {
        "nudge" => DetectorDecision::Nudge,
        "silent" => DetectorDecision::Silent,
        "vault" => DetectorDecision::Vault,
        _ => DetectorDecision::Silent, // unknown → treat as silent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::briefing::{ActivitySnapshot, FocusMode};
    use crate::llm::{LlmError, LlmResponse};
    use async_trait::async_trait;

    fn make_briefing_json(app: &str, title: &str, patterns: &str) -> String {
        let b = Briefing {
            ts: 1000,
            active_mode: Some(FocusMode::Coding),
            right_now: ActivitySnapshot {
                app: app.to_string(),
                title: Some(title.to_string()),
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

    fn make_decision(id: i64, ts: i64, decision: &str, briefing_json: &str) -> DecisionRow {
        DecisionRow {
            id,
            ts,
            trigger: "heartbeat".to_string(),
            decision: decision.to_string(),
            reasoning: "test".to_string(),
            nudge_style: None,
            nudge_message: None,
            briefing_json: briefing_json.to_string(),
            patterns_hash: "test_hash".to_string(),
            prompt_version: "detector.v1".to_string(),
            duration_ms: 100,
        }
    }

    fn make_correction(id: i64, decision_id: i64, original: &str) -> CorrectionRow {
        CorrectionRow {
            id,
            ts: 1000,
            decision_id,
            original_decision: original.to_string(),
            user_verdict: "wrong".to_string(),
            ctx_snapshot: "{}".to_string(),
            patterns_hash: "test_hash".to_string(),
            status: "pending".to_string(),
        }
    }

    /// Mock LLM that always returns "silent"
    struct AlwaysSilentLlm;

    #[async_trait]
    impl LlmBackend for AlwaysSilentLlm {
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

    /// Mock LLM that always returns "nudge"
    struct AlwaysNudgeLlm;

    #[async_trait]
    impl LlmBackend for AlwaysNudgeLlm {
        async fn complete(
            &self,
            _prompt: &str,
            _grammar: &str,
            _n_predict: u32,
            _temperature: f32,
        ) -> Result<LlmResponse, LlmError> {
            Ok(LlmResponse {
                content: r#"{"decision":"nudge","reasoning":"drift","nudge_style":"gentle","nudge_message":"hey","vault_category":null,"patterns_cited":[]}"#.to_string(),
                model: Some("test".to_string()),
            })
        }
    }

    #[test]
    fn test_curator_passes_logic() {
        assert!(curator_passes(&EvalResult {
            events_replayed: 10,
            decisions_changed: 3,
            regressions: 0,
            passed: true,
            rationale: String::new(),
            duration_ms: 0,
        }));

        // regressions > 0 → fail
        assert!(!curator_passes(&EvalResult {
            events_replayed: 10,
            decisions_changed: 3,
            regressions: 1,
            passed: false,
            rationale: String::new(),
            duration_ms: 0,
        }));

        // no decisions changed → fail (cosmetic-only)
        assert!(!curator_passes(&EvalResult {
            events_replayed: 10,
            decisions_changed: 0,
            regressions: 0,
            passed: false,
            rationale: String::new(),
            duration_ms: 0,
        }));
    }

    #[test]
    fn test_parse_decision() {
        assert_eq!(parse_decision("Nudge"), DetectorDecision::Nudge);
        assert_eq!(parse_decision("nudge"), DetectorDecision::Nudge);
        assert_eq!(parse_decision("Silent"), DetectorDecision::Silent);
        assert_eq!(parse_decision("Vault"), DetectorDecision::Vault);
        assert_eq!(parse_decision("unknown"), DetectorDecision::Silent);
    }

    #[test]
    fn test_select_samples_includes_corrections() {
        let briefing = make_briefing_json("code.exe", "main.rs", "old patterns");
        let decisions: Vec<DecisionRow> = (0..20)
            .map(|i| make_decision(i + 1, 1000 + i * 300_000, "Silent", &briefing))
            .collect();
        let correction = make_correction(1, 5, "Silent"); // correction on decision #5
        let correction_map: HashMap<i64, &CorrectionRow> =
            vec![(5, &correction)].into_iter().collect();

        let samples = select_samples(&decisions, &correction_map, 5);
        // Must include decision #5 (has correction)
        assert!(samples.iter().any(|d| d.id == 5));
        assert!(samples.len() <= 5);
    }

    #[tokio::test]
    async fn test_replay_empty_decisions() {
        let llm = AlwaysSilentLlm;
        let result = replay(&[], &[], "candidate", "profile", &llm).await.unwrap();
        assert_eq!(result.events_replayed, 0);
        assert!(result.passed);
    }

    #[tokio::test]
    async fn test_replay_detects_decision_change() {
        // Original decisions are all "Nudge", but mock LLM returns "Silent"
        let briefing = make_briefing_json("chrome.exe", "Twitter", "old patterns");
        let decisions = vec![
            make_decision(1, 1000, "Nudge", &briefing),
            make_decision(2, 2000, "Nudge", &briefing),
        ];

        let llm = AlwaysSilentLlm;
        let result = replay(&decisions, &[], "new patterns", "profile", &llm)
            .await
            .unwrap();

        assert_eq!(result.events_replayed, 2);
        assert_eq!(result.decisions_changed, 2);
        assert_eq!(result.regressions, 0);
        assert!(result.passed); // changed > 0, regressions == 0
    }

    #[tokio::test]
    async fn test_replay_detects_regression() {
        // Original decision was Nudge, user corrected it. Mock LLM still returns Nudge.
        // This means candidate patterns didn't fix the problem → regression.
        let briefing = make_briefing_json("chrome.exe", "Twitter", "old patterns");
        let decisions = vec![make_decision(1, 1000, "Nudge", &briefing)];
        let corrections = vec![make_correction(1, 1, "Nudge")]; // user said the nudge was wrong

        let llm = AlwaysNudgeLlm; // still nudges → regression
        let result = replay(&decisions, &corrections, "new patterns", "profile", &llm)
            .await
            .unwrap();

        assert_eq!(result.events_replayed, 1);
        assert_eq!(result.decisions_changed, 0); // still Nudge, no change
        assert_eq!(result.regressions, 1); // user corrected + still same answer
        assert!(!result.passed);
    }

    #[tokio::test]
    async fn test_replay_no_regression_when_fixed() {
        // Original was Nudge, user corrected. Mock LLM returns Silent (fixed).
        let briefing = make_briefing_json("chrome.exe", "Twitter", "old patterns");
        let decisions = vec![make_decision(1, 1000, "Nudge", &briefing)];
        let corrections = vec![make_correction(1, 1, "Nudge")];

        let llm = AlwaysSilentLlm; // now silent → fixed!
        let result = replay(&decisions, &corrections, "new patterns", "profile", &llm)
            .await
            .unwrap();

        assert_eq!(result.events_replayed, 1);
        assert_eq!(result.decisions_changed, 1);
        assert_eq!(result.regressions, 0);
        assert!(result.passed);
    }

    // ------------------------------------------------------------------
    // reflector_passes tests
    // ------------------------------------------------------------------

    #[test]
    fn test_reflector_passes_clean() {
        // 2 out of 20 changed (10%) → Pass
        assert_eq!(
            reflector_passes(&EvalResult {
                events_replayed: 20,
                decisions_changed: 2,
                regressions: 0,
                passed: true,
                rationale: String::new(),
                duration_ms: 0,
            }),
            ReflectorEvalOutcome::Pass
        );
    }

    #[test]
    fn test_reflector_passes_borderline() {
        // 4 out of 20 changed (20%) → Borderline
        assert_eq!(
            reflector_passes(&EvalResult {
                events_replayed: 20,
                decisions_changed: 4,
                regressions: 0,
                passed: true,
                rationale: String::new(),
                duration_ms: 0,
            }),
            ReflectorEvalOutcome::Borderline
        );
    }

    #[test]
    fn test_reflector_passes_exactly_15_pct() {
        // 3 out of 20 = 15% exactly → Borderline (>= 0.15)
        assert_eq!(
            reflector_passes(&EvalResult {
                events_replayed: 20,
                decisions_changed: 3,
                regressions: 0,
                passed: true,
                rationale: String::new(),
                duration_ms: 0,
            }),
            ReflectorEvalOutcome::Borderline
        );
    }

    #[test]
    fn test_reflector_passes_fail() {
        // Regressions > 0 → Fail regardless of ratio
        assert_eq!(
            reflector_passes(&EvalResult {
                events_replayed: 20,
                decisions_changed: 1,
                regressions: 1,
                passed: false,
                rationale: String::new(),
                duration_ms: 0,
            }),
            ReflectorEvalOutcome::Fail
        );
    }

    #[test]
    fn test_reflector_passes_zero_replays() {
        // No data → Pass (nothing to regress on)
        assert_eq!(
            reflector_passes(&EvalResult {
                events_replayed: 0,
                decisions_changed: 0,
                regressions: 0,
                passed: true,
                rationale: String::new(),
                duration_ms: 0,
            }),
            ReflectorEvalOutcome::Pass
        );
    }
}
