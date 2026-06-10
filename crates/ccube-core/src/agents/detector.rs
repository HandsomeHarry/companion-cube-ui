// Detector agent — Phase 4 implementation (v1) + Phase 8 two-step pipeline (v2).

use crate::briefing::{
    AnnotatedEntry, AnnotatedTimeline, Briefing, BriefingV2, DetectorDecision,
    DetectorOutput, DetectorV2Output,
};
use crate::llm::{LlmBackend, LlmError};

/// Prompt template version, logged with every decision.
pub const PROMPT_VERSION: &str = "detector.v1";

/// V2 prompt version (Phase 8 two-step pipeline).
pub const PROMPT_VERSION_V2: &str = "detector.v2";

/// GBNF grammar that constrains llama.cpp to produce valid DetectorOutput JSON.
pub const DETECTOR_GRAMMAR: &str = r#"
root ::= "{" ws
  "\"decision\"" ws ":" ws decision "," ws
  "\"reasoning\"" ws ":" ws string "," ws
  "\"nudge_style\"" ws ":" ws nullable-nudge-style "," ws
  "\"nudge_message\"" ws ":" ws nullable-string "," ws
  "\"vault_category\"" ws ":" ws nullable-string "," ws
  "\"patterns_cited\"" ws ":" ws int-array
  ws "}"

decision ::= "\"nudge\"" | "\"silent\"" | "\"vault\""
nudge-style ::= "\"gentle\"" | "\"direct\"" | "\"vault_offer\""
nullable-nudge-style ::= nudge-style | "null"
nullable-string ::= string | "null"

int-array ::= "[]" | "[" ws int ( "," ws int )* ws "]"
int ::= [0-9]+

string ::= "\"" chars "\""
chars ::= "" | char chars
char ::= [^"\\] | "\\" escape
escape ::= "\"" | "\\" | "/" | "b" | "f" | "n" | "r" | "t"

ws ::= | " " | "\n" | "\r" | "\t"
"#;

/// The JSON schema description embedded in the prompt.
const SCHEMA_DESC: &str = r#"{
  "decision": "nudge" | "silent" | "vault",
  "reasoning": "one sentence",
  "nudge_style": "gentle" | "direct" | "vault_offer" | null,
  "nudge_message": "string or null",
  "vault_category": "string or null",
  "patterns_cited": [line_indices]
}"#;

/// Render the detector prompt by substituting placeholders in the template.
///
/// Uses a single-pass replacement approach so that user-provided content
/// (profile, patterns, titles) cannot collide with placeholder names.
/// For example, if `profile` contains the literal text `{patterns}`, it will
/// appear verbatim in the output rather than being replaced by patterns content.
pub fn render_prompt(briefing: &Briefing) -> String {
    let template = include_str!("../prompts/detector.v1.md");

    let active_mode = match &briefing.active_mode {
        Some(m) => format!("{:?}", m),
        None => "Unspecified".to_string(),
    };

    let right_now_title = briefing.right_now.title.as_deref().unwrap_or("(no title)");

    let (just_before_app, just_before_title) = match &briefing.just_before {
        Some(s) => (s.app.as_str(), s.title.as_deref().unwrap_or("(no title)")),
        None => ("none", "none"),
    };

    let past_hour = if briefing.past_hour.is_empty() {
        "no activity".to_string()
    } else {
        briefing
            .past_hour
            .iter()
            .map(|a| {
                let mins = a.total_ms / 60_000;
                let titles = if a.top_titles.is_empty() {
                    "(no titles)".to_string()
                } else {
                    a.top_titles.join(", ")
                };
                format!("{} ({}m): {}", a.app, mins, titles)
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let calendar = briefing.calendar_hint.as_deref().unwrap_or("no event");

    let vault_today = if briefing.vault_today.is_empty() {
        "none".to_string()
    } else {
        briefing
            .vault_today
            .iter()
            .map(|v| format!("[{}] {}", v.category, v.summary))
            .collect::<Vec<_>>()
            .join(", ")
    };

    // Build a replacement table: placeholder -> value
    let replacements: &[(&str, &str)] = &[
        ("{profile}", &briefing.profile_snippet),
        ("{patterns}", &briefing.patterns_snippet),
        ("{active_mode}", &active_mode),
        ("{right_now.app}", &briefing.right_now.app),
        ("{right_now.title}", right_now_title),
        (
            "{right_now.duration_ms}",
            // We need an owned string but the slice borrows &str, so we
            // handle this specially below via a pre-formatted string.
            "",
        ),
        ("{just_before.app}", just_before_app),
        ("{just_before.title}", just_before_title),
        ("{past_hour}", &past_hour),
        ("{calendar_hint}", calendar),
        ("{vault_today}", &vault_today),
        ("{schema}", SCHEMA_DESC),
    ];

    let duration_str = briefing.right_now.duration_ms.to_string();

    // Single-pass scan using char_indices for UTF-8 safety.
    // We check byte-level '{' to find placeholder candidates, then match
    // against the remaining &str slice (which is always valid UTF-8).
    let mut result = String::with_capacity(template.len());
    let mut i = 0;
    while i < template.len() {
        if template.as_bytes()[i] == b'{' {
            let remaining = &template[i..];
            // Special-case duration_ms since it needs an owned string
            if remaining.starts_with("{right_now.duration_ms}") {
                result.push_str(&duration_str);
                i += "{right_now.duration_ms}".len();
                continue;
            }
            let mut matched = false;
            for &(placeholder, value) in replacements {
                if placeholder == "{right_now.duration_ms}" {
                    continue; // handled above
                }
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
            // Advance by one full UTF-8 character
            let ch = &template[i..];
            let c = ch.chars().next().unwrap();
            result.push(c);
            i += c.len_utf8();
        }
    }

    result
}

/// Run the detector: render prompt, call LLM, parse response.
///
/// On any failure (LLM unreachable, bad response, parse error), returns a
/// Silent fallback decision — the detector never panics or crashes the daemon.
pub async fn run(briefing: &Briefing, llm: &dyn LlmBackend) -> DetectorOutput {
    let prompt = render_prompt(briefing);

    match llm.complete(&prompt, DETECTOR_GRAMMAR, 512, 0.2).await {
        Ok(resp) => match parse_step2_lenient(&resp.content) {
            Ok(output) => output,
            Err(e) => {
                tracing::warn!(error = %e, "detector: failed to parse LLM response");
                silent_fallback("LLM response parse error")
            }
        },
        Err(LlmError::Unreachable(msg)) => {
            tracing::warn!(error = %msg, "detector: LLM unreachable");
            silent_fallback("LLM unreachable")
        }
        Err(LlmError::BadResponse(msg)) => {
            tracing::warn!(error = %msg, "detector: LLM bad response");
            silent_fallback("LLM bad response")
        }
    }
}

fn silent_fallback(reason: &str) -> DetectorOutput {
    DetectorOutput {
        decision: DetectorDecision::Silent,
        reasoning: reason.to_string(),
        nudge_style: None,
        nudge_message: None,
        vault_category: None,
        patterns_cited: vec![],
    }
}

// ---------------------------------------------------------------------------
// V2 two-step pipeline (Phase 8)
// ---------------------------------------------------------------------------

/// GBNF grammar for Step 1 annotation output.
pub const ANNOTATION_GRAMMAR: &str = r#"
root ::= "{" ws
  "\"annotations\"" ws ":" ws annotation-array ( "," ws "\"rhythm_notes\"" ws ":" ws nullable-string )?
  ws "}"

annotation-array ::= "[]" | "[" ws annotation ( "," ws annotation )* ws "]"
annotation ::= "{" ws
  "\"event_ts\"" ws ":" ws int "," ws
  "\"intent\"" ws ":" ws string ( "," ws "\"intent_reasoning\"" ws ":" ws nullable-string )?
  ws "}"

nullable-string ::= string | "null"
int ::= [0-9]+
string ::= "\"" chars "\""
chars ::= "" | char chars
char ::= [^"\\] | "\\" escape
escape ::= "\"" | "\\" | "/" | "b" | "f" | "n" | "r" | "t"
ws ::= | " " | "\n" | "\r" | "\t"
"#;

/// JSON schema description embedded in the Step 1 prompt.
const STEP1_SCHEMA_DESC: &str = r#"{
  "annotations": [
    {"event_ts": <ts>, "intent": "<guess>", "intent_reasoning": "<why?>"}
    ...
  ],
  "rhythm_notes": "overall rhythm pattern or null"
}"#;

/// JSON schema description embedded in the Step 2 prompt (same as v1 output).
const STEP2_SCHEMA_DESC: &str = r#"{
  "decision": "nudge" | "silent" | "vault",
  "reasoning": "one sentence",
  "nudge_style": "gentle" | "direct" | "vault_offer" | null,
  "nudge_message": "string or null",
  "vault_category": "string or null",
  "patterns_cited": [line_indices]
}"#;

/// Format timeline events for the Step 1 prompt.
fn format_timeline_events(events: &[crate::briefing::TimelineEvent]) -> String {
    if events.is_empty() {
        return "no activity this window".to_string();
    }

    events
        .iter()
        .map(|e| {
            let ts_hms = ts_hms(e.ts);
            let dur_secs = e.duration_ms / 1000;
            let ocr_line = e
                .ocr_text
                .as_ref()
                .map(|t| format!(" | ocr: \"{}\"", t.replace('\n', " | ")))
                .unwrap_or_default();
            let url_line = e
                .url
                .as_ref()
                .map(|u| format!(" | url: {}", u))
                .unwrap_or_default();
            let title = e.title.as_deref().unwrap_or("(no title)");
            // event_ts is printed explicitly so the model can echo the exact
            // integer in its annotations (Ollama cannot enforce the GBNF
            // grammar, so the schema alone doesn't guarantee an int).
            format!(
                "  [{ts_hms}] event_ts={ts} {app} | {title} | {dur_secs}s | mode: {mode}{ocr_line}{url_line}",
                ts = e.ts,
                app = e.app,
                mode = e.mode,
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Format an epoch-ms timestamp as the `HH:MM:SS` label used in prompts.
fn ts_hms(ts_ms: i64) -> String {
    let secs = ts_ms / 1000;
    let h = (secs / 3600) % 24;
    let m = (secs / 60) % 60;
    let s = secs % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

/// Parse the step-1 annotation JSON leniently.
///
/// Grammars are a llama.cpp feature — Ollama ignores them — so small local
/// models emit `event_ts` in whatever shape they fancy: epoch ms, epoch
/// seconds, a digit string, or the `HH:MM:SS` label shown in the prompt.
/// Each annotation is resolved back to a real event timestamp; entries that
/// match no event are dropped (annotations are best-effort context for
/// step 2, not load-bearing data).
fn parse_step1_lenient(
    content: &str,
    events: &[crate::briefing::TimelineEvent],
) -> Result<AnnotatedTimeline, serde_json::Error> {
    let v: serde_json::Value = serde_json::from_str(content)?;

    let rhythm_notes = v
        .get("rhythm_notes")
        .and_then(|r| r.as_str())
        .map(String::from);

    let resolve = |raw: &serde_json::Value| -> Option<i64> {
        let from_num = |n: i64| {
            if events.iter().any(|e| e.ts == n) {
                Some(n)
            } else {
                // Model may echo epoch seconds instead of milliseconds.
                events.iter().find(|e| e.ts / 1000 == n).map(|e| e.ts)
            }
        };
        match raw {
            serde_json::Value::Number(n) => n.as_i64().and_then(from_num),
            serde_json::Value::String(s) => {
                if let Ok(n) = s.trim().parse::<i64>() {
                    from_num(n)
                } else {
                    // "HH:MM:SS" label from the prompt
                    events.iter().find(|e| ts_hms(e.ts) == s.trim()).map(|e| e.ts)
                }
            }
            _ => None,
        }
    };

    let annotations = v
        .get("annotations")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|a| {
                    let intent = a.get("intent")?.as_str()?.to_string();
                    let event_ts = resolve(a.get("event_ts")?)?;
                    let intent_reasoning = a
                        .get("intent_reasoning")
                        .and_then(|r| r.as_str())
                        .map(String::from);
                    Some(AnnotatedEntry {
                        event_ts,
                        intent,
                        intent_reasoning,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(AnnotatedTimeline {
        annotations,
        rhythm_notes,
    })
}

/// Parse the step-2 verdict JSON leniently (same rationale as step 1:
/// Ollama can't enforce the grammar). Sanitizes the shapes small models
/// actually get wrong — non-integer `patterns_cited` entries (timestamps,
/// strings) are dropped, and omitted optional fields become null — then
/// hands off to the normal typed deserialization.
fn parse_step2_lenient(content: &str) -> Result<DetectorOutput, serde_json::Error> {
    let mut v: serde_json::Value = serde_json::from_str(content)?;

    if let Some(obj) = v.as_object_mut() {
        let cited: Vec<u64> = obj
            .get("patterns_cited")
            .and_then(|p| p.as_array())
            .map(|arr| arr.iter().filter_map(|x| x.as_u64()).collect())
            .unwrap_or_default();
        obj.insert("patterns_cited".to_string(), serde_json::json!(cited));

        obj.entry("reasoning").or_insert(serde_json::json!(""));
        for key in ["nudge_style", "nudge_message", "vault_category"] {
            obj.entry(key).or_insert(serde_json::Value::Null);
        }
    }

    serde_json::from_value(v)
}

/// Render the Step 1 prompt (intent annotation).
pub fn render_step1_prompt(briefing: &BriefingV2) -> String {
    let template = include_str!("../prompts/detector_v2_step1.md");
    let events_formatted = format_timeline_events(&briefing.events);

    let replacements: &[(&str, &str)] = &[
        ("{profile}", &briefing.memory.profile),
        ("{patterns}", &briefing.memory.patterns),
        ("{events}", &events_formatted),
        ("{schema}", STEP1_SCHEMA_DESC),
    ];

    let switch_count = briefing.metrics.switch_count.to_string();
    let avg_duration = briefing.metrics.avg_session_duration_ms.to_string();
    let is_afk = if briefing.metrics.is_currently_afk {
        "yes"
    } else {
        "no"
    };
    let transitioned_afk = if briefing.metrics.transitioned_afk_to_active {
        "yes"
    } else {
        "no"
    };

    let mut result = String::with_capacity(template.len());
    let mut i = 0;
    while i < template.len() {
        if template.as_bytes()[i] == b'{' {
            let remaining = &template[i..];
            // Handle special-cased metrics placeholders
            if remaining.starts_with("{switch_count}") {
                result.push_str(&switch_count);
                i += "{switch_count}".len();
                continue;
            }
            if remaining.starts_with("{avg_duration}") {
                result.push_str(&avg_duration);
                i += "{avg_duration}".len();
                continue;
            }
            if remaining.starts_with("{is_afk}") {
                result.push_str(is_afk);
                i += "{is_afk}".len();
                continue;
            }
            if remaining.starts_with("{transitioned_afk}") {
                result.push_str(transitioned_afk);
                i += "{transitioned_afk}".len();
                continue;
            }
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

/// Format annotated events for the Step 2 prompt.
fn format_annotated_events(events: &[crate::briefing::TimelineEvent], annotations: &[AnnotatedEntry]) -> String {
    if events.is_empty() {
        return "no activity this window".to_string();
    }

    events
        .iter()
        .map(|e| {
            let ts_hms = ts_hms(e.ts);
            let dur_secs = e.duration_ms / 1000;
            let title = e.title.as_deref().unwrap_or("(no title)");

            let annotation = annotations
                .iter()
                .find(|a| a.event_ts == e.ts)
                .map(|a| {
                    let reason = a
                        .intent_reasoning
                        .as_deref()
                        .map(|r| format!(" ({r})"))
                        .unwrap_or_default();
                    format!(" → intent: \"{}\"{}", a.intent, reason)
                })
                .unwrap_or_default();

            format!(
                "  [{ts_hms}] {app} | {title} | {dur_secs}s | mode: {mode}{annotation}",
                app = e.app,
                mode = e.mode,
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Render the Step 2 prompt (verdict).
pub fn render_step2_prompt(
    briefing: &BriefingV2,
    annotations: &[AnnotatedEntry],
    rhythm_notes: Option<&str>,
) -> String {
    let template = include_str!("../prompts/detector_v2_step2.md");
    let annotated_formatted = format_annotated_events(&briefing.events, annotations);
    let rhythm = rhythm_notes.unwrap_or("no clear rhythm pattern detected");

    let replacements: &[(&str, &str)] = &[
        ("{profile}", &briefing.memory.profile),
        ("{patterns}", &briefing.memory.patterns),
        ("{annotated_events}", &annotated_formatted),
        ("{rhythm_notes}", rhythm),
        ("{schema}", STEP2_SCHEMA_DESC),
    ];

    let switch_count = briefing.metrics.switch_count.to_string();
    let avg_duration = briefing.metrics.avg_session_duration_ms.to_string();
    let is_afk = if briefing.metrics.is_currently_afk {
        "yes"
    } else {
        "no"
    };
    let transitioned_afk = if briefing.metrics.transitioned_afk_to_active {
        "yes"
    } else {
        "no"
    };

    let mut result = String::with_capacity(template.len());
    let mut i = 0;
    while i < template.len() {
        if template.as_bytes()[i] == b'{' {
            let remaining = &template[i..];
            if remaining.starts_with("{switch_count}") {
                result.push_str(&switch_count);
                i += "{switch_count}".len();
                continue;
            }
            if remaining.starts_with("{avg_duration}") {
                result.push_str(&avg_duration);
                i += "{avg_duration}".len();
                continue;
            }
            if remaining.starts_with("{is_afk}") {
                result.push_str(is_afk);
                i += "{is_afk}".len();
                continue;
            }
            if remaining.starts_with("{transitioned_afk}") {
                result.push_str(transitioned_afk);
                i += "{transitioned_afk}".len();
                continue;
            }
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

/// Run the v2 two-step detector pipeline.
///
/// Step 1: Annotate each event with inferred user intent.
/// Step 2: Decide verdict based on annotated timeline.
///
/// On any LLM failure, returns a Silent fallback with empty annotations.
pub async fn run_v2(briefing: &BriefingV2, llm: &dyn LlmBackend) -> DetectorV2Output {
    // Step 1: Intent annotation
    let step1_prompt = render_step1_prompt(briefing);

    let (annotations, rhythm_notes) = match llm
        .complete(&step1_prompt, ANNOTATION_GRAMMAR, 2048, 0.2)
        .await
    {
        Ok(resp) => match parse_step1_lenient(&resp.content, &briefing.events) {
            Ok(timeline) => (timeline.annotations, timeline.rhythm_notes),
            Err(e) => {
                tracing::warn!(error = %e, "detector_v2: failed to parse step1 annotation");
                return silent_fallback_v2("step1 parse error", vec![], None);
            }
        },
        Err(e) => {
            tracing::warn!(error = %e, "detector_v2: step1 LLM call failed");
            return silent_fallback_v2("step1 LLM error", vec![], None);
        }
    };

    // Step 2: Verdict
    let step2_prompt = render_step2_prompt(briefing, &annotations, rhythm_notes.as_deref());

    match llm.complete(&step2_prompt, DETECTOR_GRAMMAR, 512, 0.2).await {
        Ok(resp) => match parse_step2_lenient(&resp.content) {
            Ok(output) => DetectorV2Output {
                decision: output.decision,
                reasoning: output.reasoning,
                nudge_style: output.nudge_style,
                nudge_message: output.nudge_message,
                vault_category: output.vault_category,
                patterns_cited: output.patterns_cited,
                annotations,
                rhythm_notes,
            },
            Err(e) => {
                tracing::warn!(error = %e, "detector_v2: failed to parse step2 verdict");
                silent_fallback_v2("step2 parse error", annotations, rhythm_notes)
            }
        },
        Err(e) => {
            tracing::warn!(error = %e, "detector_v2: step2 LLM call failed");
            silent_fallback_v2("step2 LLM error", annotations, rhythm_notes)
        }
    }
}

fn silent_fallback_v2(
    reason: &str,
    annotations: Vec<AnnotatedEntry>,
    rhythm_notes: Option<String>,
) -> DetectorV2Output {
    DetectorV2Output {
        decision: DetectorDecision::Silent,
        reasoning: reason.to_string(),
        nudge_style: None,
        nudge_message: None,
        vault_category: None,
        patterns_cited: vec![],
        annotations,
        rhythm_notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::briefing::{ActivitySnapshot, FocusMode, NudgeStyle};
    use crate::llm::LlmResponse;
    use async_trait::async_trait;

    fn test_briefing() -> Briefing {
        Briefing {
            ts: 1000000,
            active_mode: Some(FocusMode::Coding),
            right_now: ActivitySnapshot {
                app: "Code.exe".to_string(),
                title: Some("main.rs".to_string()),
                url: None,
                duration_ms: 30000,
            },
            just_before: Some(ActivitySnapshot {
                app: "chrome.exe".to_string(),
                title: Some("Google".to_string()),
                url: None,
                duration_ms: 15000,
            }),
            past_hour: vec![],
            calendar_hint: None,
            vault_today: vec![],
            profile_snippet: "I am a developer".to_string(),
            patterns_snippet: "§ coding in rust is on-task".to_string(),
            patterns_hash: "abc123".to_string(),
        }
    }

    fn test_events() -> Vec<crate::briefing::TimelineEvent> {
        vec![crate::briefing::TimelineEvent {
            ts: 1749600782000, // 00:13:02 UTC
            app: "Brave Browser".to_string(),
            title: Some("YouTube".to_string()),
            ocr_text: None,
            url: None,
            duration_ms: 30000,
            mode: "Browsing".to_string(),
        }]
    }

    #[test]
    fn test_step1_lenient_int_ts() {
        let json = r#"{"annotations":[{"event_ts":1749600782000,"intent":"watching videos"}],"rhythm_notes":null}"#;
        let t = parse_step1_lenient(json, &test_events()).unwrap();
        assert_eq!(t.annotations.len(), 1);
        assert_eq!(t.annotations[0].event_ts, 1749600782000);
    }

    #[test]
    fn test_step1_lenient_string_digits_and_seconds() {
        // epoch seconds as a string — both quirks at once
        let json = r#"{"annotations":[{"event_ts":"1749600782","intent":"watching videos"}]}"#;
        let t = parse_step1_lenient(json, &test_events()).unwrap();
        assert_eq!(t.annotations[0].event_ts, 1749600782000);
    }

    #[test]
    fn test_step1_lenient_hms_label() {
        let json = r#"{"annotations":[{"event_ts":"00:13:02","intent":"watching videos","intent_reasoning":"title says YouTube"}]}"#;
        let t = parse_step1_lenient(json, &test_events()).unwrap();
        assert_eq!(t.annotations[0].event_ts, 1749600782000);
        assert_eq!(
            t.annotations[0].intent_reasoning.as_deref(),
            Some("title says YouTube")
        );
    }

    #[test]
    fn test_step1_lenient_unmatched_dropped() {
        let json = r#"{"annotations":[{"event_ts":"99:99:99","intent":"???"},{"event_ts":"00:13:02","intent":"ok"}]}"#;
        let t = parse_step1_lenient(json, &test_events()).unwrap();
        assert_eq!(t.annotations.len(), 1);
        assert_eq!(t.annotations[0].intent, "ok");
    }

    #[test]
    fn test_step2_lenient_garbage_patterns_cited() {
        // timestamps-as-strings in patterns_cited, optional fields omitted
        let json = r#"{"decision":"silent","reasoning":"working normally","patterns_cited":["08:18:32", 2, "x"]}"#;
        let out = parse_step2_lenient(json).unwrap();
        assert_eq!(out.decision, DetectorDecision::Silent);
        assert_eq!(out.patterns_cited, vec![2]);
        assert!(out.nudge_message.is_none());
    }

    #[test]
    fn test_step1_prompt_includes_event_ts() {
        let formatted = format_timeline_events(&test_events());
        assert!(formatted.contains("event_ts=1749600782000"));
    }

    struct MockLlm {
        response: Result<String, LlmError>,
    }

    #[async_trait]
    impl LlmBackend for MockLlm {
        async fn complete(
            &self,
            _prompt: &str,
            _grammar: &str,
            _n_predict: u32,
            _temperature: f32,
        ) -> Result<LlmResponse, LlmError> {
            match &self.response {
                Ok(content) => Ok(LlmResponse {
                    content: content.clone(),
                    model: Some("test-model".to_string()),
                }),
                Err(_) => Err(LlmError::Unreachable("mock down".into())),
            }
        }
    }

    #[tokio::test]
    async fn test_happy_path_silent() {
        let llm = MockLlm {
            response: Ok(r#"{"decision":"silent","reasoning":"user is coding in Rust, on-task","nudge_style":null,"nudge_message":null,"vault_category":null,"patterns_cited":[0]}"#.to_string()),
        };
        let output = run(&test_briefing(), &llm).await;
        assert_eq!(output.decision, DetectorDecision::Silent);
        assert!(output.reasoning.contains("coding"));
        assert_eq!(output.patterns_cited, vec![0]);
    }

    #[tokio::test]
    async fn test_happy_path_nudge() {
        let llm = MockLlm {
            response: Ok(r#"{"decision":"nudge","reasoning":"browsing social media","nudge_style":"gentle","nudge_message":"Looks like you drifted to social media","vault_category":null,"patterns_cited":[]}"#.to_string()),
        };
        let output = run(&test_briefing(), &llm).await;
        assert_eq!(output.decision, DetectorDecision::Nudge);
        assert_eq!(output.nudge_style, Some(NudgeStyle::Gentle));
        assert!(output.nudge_message.is_some());
    }

    #[tokio::test]
    async fn test_llm_unreachable_returns_silent() {
        let llm = MockLlm {
            response: Err(LlmError::Unreachable("down".into())),
        };
        let output = run(&test_briefing(), &llm).await;
        assert_eq!(output.decision, DetectorDecision::Silent);
        assert_eq!(output.reasoning, "LLM unreachable");
    }

    #[tokio::test]
    async fn test_malformed_json_returns_silent() {
        let llm = MockLlm {
            response: Ok("not valid json at all".to_string()),
        };
        let output = run(&test_briefing(), &llm).await;
        assert_eq!(output.decision, DetectorDecision::Silent);
        assert_eq!(output.reasoning, "LLM response parse error");
    }

    #[test]
    fn test_prompt_render_no_placeholders_remain() {
        let prompt = render_prompt(&test_briefing());
        assert!(!prompt.contains("{profile}"));
        assert!(!prompt.contains("{patterns}"));
        assert!(!prompt.contains("{active_mode}"));
        assert!(!prompt.contains("{right_now.app}"));
        assert!(!prompt.contains("{schema}"));
        assert!(prompt.contains("I am a developer"));
        assert!(prompt.contains("Code.exe"));
    }

    #[test]
    fn test_prompt_injection_safe() {
        // Profile containing a placeholder name should NOT cause it to be
        // substituted by a later .replace() call.
        let mut b = test_briefing();
        b.profile_snippet = "Profile with {patterns} placeholder".to_string();
        b.patterns_snippet = "REAL_PATTERNS".to_string();
        let prompt = render_prompt(&b);
        // The literal "{patterns}" from profile should appear in the output,
        // and the real patterns should also appear separately.
        assert!(prompt.contains("{patterns}"));
        assert!(prompt.contains("REAL_PATTERNS"));
    }
}
