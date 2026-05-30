use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::db::EventRow;
use crate::focus_mode;
use crate::memory;

/// The core data type consumed by the detector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Briefing {
    pub ts: i64,
    pub active_mode: Option<FocusMode>,
    pub right_now: ActivitySnapshot,
    pub just_before: Option<ActivitySnapshot>,
    pub past_hour: Vec<ActivityAggregate>,
    pub calendar_hint: Option<String>,
    pub vault_today: Vec<VaultEntry>,
    pub profile_snippet: String,
    pub patterns_snippet: String,
    pub patterns_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FocusMode {
    Coding,
    Writing,
    VideoProduction,
    Unspecified,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivitySnapshot {
    pub app: String,
    pub title: Option<String>,
    pub url: Option<String>,
    pub duration_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityAggregate {
    pub app: String,
    pub category: Option<String>,
    pub total_ms: i64,
    pub top_titles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultEntry {
    pub ts: i64,
    pub category: String,
    pub summary: String,
}

/// The detector's output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectorOutput {
    pub decision: DetectorDecision,
    pub reasoning: String,
    pub nudge_style: Option<NudgeStyle>,
    pub nudge_message: Option<String>,
    pub vault_category: Option<String>,
    pub patterns_cited: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DetectorDecision {
    Nudge,
    Silent,
    Vault,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NudgeStyle {
    Gentle,
    Direct,
    VaultOffer,
}

/// Curator output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuratorOutput {
    pub correction_verdicts: Vec<CorrectionVerdict>,
    pub proposed_adds: Vec<PatternAdd>,
    pub proposed_replaces: Vec<PatternReplace>,
    pub needs_reflection: bool,
    pub overall_rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrectionVerdict {
    pub correction_id: i64,
    pub verdict: String,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternAdd {
    pub text: String,
    pub supporting_correction_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternReplace {
    pub old_text: String,
    pub new_text: String,
    pub rationale: String,
}

/// Reflector output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectorOutput {
    pub new_patterns_md: String,
    pub rationale: String,
}

// ---------------------------------------------------------------------------
// Briefing builder — pure function, no I/O
// ---------------------------------------------------------------------------

/// Build a Briefing from raw event data and frozen memory.
///
/// This is a pure function: all inputs are provided by the caller.
/// Maximum age (ms) for an event to be considered "currently active."
/// If the most recent app_focus event is older than this relative to now_ms,
/// its duration is NOT extrapolated to the present — the daemon was likely offline.
/// Matches the idle threshold (5 minutes).
const MAX_LIVENESS_GAP_MS: i64 = 300_000;

/// `now_ms` is the current timestamp in milliseconds (passed in for testability).
/// `events` should be the last hour of events, ordered by `ts` ascending.
pub fn build(
    now_ms: i64,
    events: &[EventRow],
    profile: &str,
    patterns: &str,
    vault_today: &[VaultEntry],
) -> Briefing {
    // 0. Find the most recent daemon_start sentinel — events before this are from
    //    a previous session and should never have their duration extrapolated.
    let session_start_ts = events
        .iter()
        .rev()
        .find(|e| e.kind == "daemon_start")
        .map(|e| e.ts)
        .unwrap_or(0);

    // Helper: resolve an event's effective duration.
    // - If duration_ms is set (event was finalized), use it as-is.
    // - If duration_ms is NULL (still "active"), only extrapolate to now if the
    //   event is from the current session AND within the liveness gap. Otherwise
    //   treat as 0 (stale / previous session).
    let resolve_dur = |e: &EventRow| -> i64 {
        if let Some(d) = e.duration_ms {
            return d;
        }
        // NULL duration — is this event from the current session and recent?
        let from_current_session = e.ts >= session_start_ts;
        let within_liveness = (now_ms - e.ts) <= MAX_LIVENESS_GAP_MS;
        if from_current_session && within_liveness {
            (now_ms - e.ts).max(0)
        } else {
            0
        }
    };

    // 1. Filter sub-2s events (keep events with duration_ms None = active/current)
    let filtered: Vec<&EventRow> = events
        .iter()
        .filter(|e| !matches!(e.duration_ms, Some(d) if d < 2000))
        .collect();

    // 2. Build right_now from the most recent app_focus event
    let right_now = filtered
        .iter()
        .rev()
        .find(|e| e.kind == "app_focus")
        .map(|e| {
            let dur = resolve_dur(e);
            // If the event is stale (0 duration from resolve_dur, NULL original),
            // show "unknown" rather than a misleading old app name.
            if dur == 0 && e.duration_ms.is_none() {
                ActivitySnapshot {
                    app: "unknown".to_string(),
                    title: Some("daemon was offline".to_string()),
                    url: None,
                    duration_ms: 0,
                }
            } else {
                ActivitySnapshot {
                    app: e.app.clone().unwrap_or_default(),
                    title: e.title.clone(),
                    url: None,
                    duration_ms: dur,
                }
            }
        })
        .unwrap_or(ActivitySnapshot {
            app: "unknown".to_string(),
            title: None,
            url: None,
            duration_ms: 0,
        });

    // 3. Build just_before: walk backwards from the end to find the first
    //    app_focus event with a different app name
    let just_before = filtered
        .iter()
        .rev()
        .filter(|e| e.kind == "app_focus")
        .find(|e| e.app.as_deref().unwrap_or("") != right_now.app)
        .map(|e| ActivitySnapshot {
            app: e.app.clone().unwrap_or_default(),
            title: e.title.clone(),
            url: None,
            duration_ms: resolve_dur(e),
        });

    // 4. Build past_hour aggregates: group by app, sum durations, top 3 titles
    let mut app_data: HashMap<String, (i64, Vec<String>)> = HashMap::new();
    for e in &filtered {
        if e.kind != "app_focus" {
            continue;
        }
        let app = e.app.clone().unwrap_or_default();
        let dur = resolve_dur(e);
        let entry = app_data.entry(app).or_insert_with(|| (0, Vec::new()));
        entry.0 += dur;
        if let Some(ref t) = e.title
            && !t.is_empty()
            && !entry.1.contains(t)
        {
            entry.1.push(t.clone());
        }
    }

    let mut past_hour: Vec<ActivityAggregate> = app_data
        .into_iter()
        .map(|(app, (total_ms, titles))| {
            let top_titles: Vec<String> = titles.into_iter().take(3).collect();
            ActivityAggregate {
                app,
                category: None,
                total_ms,
                top_titles,
            }
        })
        .collect();
    past_hour.sort_by(|a, b| b.total_ms.cmp(&a.total_ms));

    // 5. Infer active_mode from right_now
    let active_mode = Some(focus_mode::infer_focus_mode(
        &right_now.app,
        right_now.title.as_deref(),
        None,
        None,
    ));

    // 6. Assemble
    Briefing {
        ts: now_ms,
        active_mode,
        right_now,
        just_before,
        past_hour,
        calendar_hint: None,
        vault_today: vault_today.to_vec(),
        profile_snippet: profile.to_string(),
        patterns_snippet: patterns.to_string(),
        patterns_hash: memory::patterns_hash(patterns),
    }
}

// ---------------------------------------------------------------------------
// BriefingV2 builder — v2 pipeline (Phase 8)
// ---------------------------------------------------------------------------

/// Per-event entry in the detector's timeline (Phase 8 v2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEvent {
    pub ts: i64,
    pub app: String,
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ocr_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub duration_ms: i64,
    pub mode: String,
}

/// Behavioral metrics for the 5-minute detection window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregateMetrics {
    pub switch_count: u32,
    pub avg_session_duration_ms: i64,
    pub is_currently_afk: bool,
    pub transitioned_afk_to_active: bool,
}

/// Memory context for the v2 detector (Phase 8).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryContext {
    pub profile: String,
    pub patterns: String,
    pub patterns_hash: String,
}

/// The v2 briefing — what build_v2() produces.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BriefingV2 {
    pub ts: i64,
    pub events: Vec<TimelineEvent>,
    pub metrics: AggregateMetrics,
    pub memory: MemoryContext,
    pub vault_today: Vec<VaultEntry>,
}

/// Step 1 output: annotated timeline with per-event intent guesses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotatedTimeline {
    pub annotations: Vec<AnnotatedEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rhythm_notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotatedEntry {
    pub event_ts: i64,
    pub intent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent_reasoning: Option<String>,
}

/// Step 2 output: final detector decision (v2 format).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectorV2Output {
    pub decision: DetectorDecision,
    pub reasoning: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nudge_style: Option<NudgeStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nudge_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vault_category: Option<String>,
    pub patterns_cited: Vec<usize>,
    pub annotations: Vec<AnnotatedEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rhythm_notes: Option<String>,
}

/// Build a BriefingV2 from raw event data and frozen memory.
///
/// This is a pure function: all inputs are provided by the caller.
/// `now_ms` is the current timestamp in milliseconds (passed in for testability).
/// `events` should be the last 5 minutes of events, ordered by `ts` ascending.
pub fn build_v2(
    now_ms: i64,
    events: &[EventRow],
    profile: &str,
    patterns: &str,
    vault_today: &[VaultEntry],
) -> BriefingV2 {
    let window_start = now_ms - 300_000; // 5 minutes

    // Helper: resolve an event's effective duration (same logic as v1 build()).
    let session_start_ts = events
        .iter()
        .rev()
        .find(|e| e.kind == "daemon_start")
        .map(|e| e.ts)
        .unwrap_or(0);

    let resolve_dur = |e: &EventRow| -> i64 {
        if let Some(d) = e.duration_ms {
            return d;
        }
        let from_current_session = e.ts >= session_start_ts;
        let within_liveness = (now_ms - e.ts) <= MAX_LIVENESS_GAP_MS;
        if from_current_session && within_liveness {
            (now_ms - e.ts).max(0)
        } else {
            0
        }
    };

    // Collect URL events (to merge nearest URL into each app_focus event).
    let url_events: Vec<&EventRow> = events
        .iter()
        .filter(|e| e.kind == "url" && e.title.is_some())
        .collect();

    // Helper: find nearest URL at or before a given timestamp.
    let nearest_url = |ts: i64| -> Option<String> {
        url_events
            .iter()
            .rev()
            .find(|e| e.ts <= ts)
            .and_then(|e| e.title.clone())
    };

    // Build timeline from app_focus events within the 5-minute window.
    let mut timeline: Vec<TimelineEvent> = events
        .iter()
        .filter(|e| e.kind == "app_focus" && e.ts >= window_start)
        .map(|e| {
            let dur = resolve_dur(e);
            let mode_str = e
                .mode
                .clone()
                .unwrap_or_else(|| "Unspecified".to_string());
            TimelineEvent {
                ts: e.ts,
                app: e.app.clone().unwrap_or_default(),
                title: e.title.clone(),
                ocr_text: e.ocr_text.clone(),
                url: nearest_url(e.ts),
                duration_ms: dur,
                mode: mode_str,
            }
        })
        .collect();

    // Ensure chronological order (should already be, but be safe).
    timeline.sort_by_key(|e| e.ts);

    // Compute aggregate metrics.
    let switch_count = timeline.len() as u32;

    let non_zero_durations: Vec<i64> = timeline
        .iter()
        .map(|e| e.duration_ms)
        .filter(|&d| d > 0)
        .collect();

    let avg_session_duration_ms = if non_zero_durations.is_empty() {
        0
    } else {
        let sum: i64 = non_zero_durations.iter().sum();
        sum / non_zero_durations.len() as i64
    };

    // Check AFK state: look at idle events within the window.
    let window_events: Vec<&EventRow> = events
        .iter()
        .filter(|e| e.ts >= window_start)
        .collect();

    let last_idle_kind = window_events
        .iter()
        .rev()
        .find(|e| e.kind == "idle_start" || e.kind == "idle_end")
        .map(|e| e.kind.as_str());

    let is_currently_afk = last_idle_kind == Some("idle_start");

    let transitioned_afk_to_active = window_events
        .iter()
        .any(|e| e.kind == "idle_end");

    let metrics = AggregateMetrics {
        switch_count,
        avg_session_duration_ms,
        is_currently_afk,
        transitioned_afk_to_active,
    };

    // Build memory context.
    let memory = MemoryContext {
        profile: profile.to_string(),
        patterns: patterns.to_string(),
        patterns_hash: memory::patterns_hash(patterns),
    };

    BriefingV2 {
        ts: now_ms,
        events: timeline,
        metrics,
        memory,
        vault_today: vault_today.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: i64, ts: i64, app: &str, title: &str, duration_ms: Option<i64>) -> EventRow {
        EventRow {
            id,
            ts,
            kind: "app_focus".to_string(),
            app: Some(app.to_string()),
            title: if title.is_empty() {
                None
            } else {
                Some(title.to_string())
            },
            duration_ms,
            mode: None,
            ocr_text: None,
            vision_desc: None,
        }
    }

    #[test]
    fn test_basic_happy_path() {
        let events = vec![
            event(1, 1000, "Code.exe", "main.rs", Some(30000)),
            event(2, 31000, "chrome.exe", "Google", Some(15000)),
            event(3, 46000, "Code.exe", "lib.rs", None),
        ];
        let b = build(50000, &events, "my profile", "my patterns", &[]);

        assert_eq!(b.right_now.app, "Code.exe");
        assert_eq!(b.right_now.title.as_deref(), Some("lib.rs"));
        assert_eq!(b.right_now.duration_ms, 4000); // 50000 - 46000
        assert_eq!(b.just_before.as_ref().unwrap().app, "chrome.exe");
        assert!(!b.past_hour.is_empty());
        assert_eq!(b.profile_snippet, "my profile");
        assert_eq!(b.patterns_snippet, "my patterns");
        assert!(!b.patterns_hash.is_empty());
    }

    #[test]
    fn test_sub_2s_filtering() {
        let events = vec![
            event(1, 1000, "Code.exe", "main.rs", Some(30000)),
            event(2, 31000, "explorer.exe", "Desktop", Some(500)), // <2s, filtered
            event(3, 31500, "chrome.exe", "Google", Some(1999)),   // <2s, filtered
            event(4, 33500, "Code.exe", "lib.rs", None),
        ];
        let b = build(40000, &events, "", "", &[]);

        // The explorer.exe and chrome.exe events should be filtered out
        assert_eq!(b.past_hour.len(), 1); // only Code.exe
        assert_eq!(b.past_hour[0].app, "Code.exe");
    }

    #[test]
    fn test_consecutive_same_app_aggregated() {
        let events = vec![
            event(1, 1000, "Code.exe", "main.rs", Some(10000)),
            event(2, 11000, "Code.exe", "lib.rs", Some(10000)),
            event(3, 21000, "Code.exe", "test.rs", None),
        ];
        let b = build(30000, &events, "", "", &[]);

        assert_eq!(b.past_hour.len(), 1);
        assert_eq!(b.past_hour[0].app, "Code.exe");
        assert_eq!(b.past_hour[0].total_ms, 29000); // 10000 + 10000 + (30000-21000)
        assert_eq!(b.past_hour[0].top_titles.len(), 3);
    }

    #[test]
    fn test_title_dedup_in_aggregates() {
        let events = vec![
            event(1, 1000, "Code.exe", "main.rs", Some(5000)),
            event(2, 6000, "Code.exe", "main.rs", Some(5000)), // dup title
            event(3, 11000, "Code.exe", "main.rs", Some(5000)), // dup title
            event(4, 16000, "Code.exe", "lib.rs", None),
        ];
        let b = build(20000, &events, "", "", &[]);

        assert_eq!(b.past_hour[0].top_titles.len(), 2); // main.rs, lib.rs (deduped)
    }

    #[test]
    fn test_top_3_title_cap() {
        let events = vec![
            event(1, 1000, "Code.exe", "a.rs", Some(5000)),
            event(2, 6000, "Code.exe", "b.rs", Some(5000)),
            event(3, 11000, "Code.exe", "c.rs", Some(5000)),
            event(4, 16000, "Code.exe", "d.rs", Some(5000)),
            event(5, 21000, "Code.exe", "e.rs", Some(5000)),
            event(6, 26000, "Code.exe", "f.rs", None),
        ];
        let b = build(30000, &events, "", "", &[]);

        assert_eq!(b.past_hour[0].top_titles.len(), 3); // capped at 3
    }

    #[test]
    fn test_single_app_no_just_before() {
        let events = vec![
            event(1, 1000, "Code.exe", "main.rs", Some(10000)),
            event(2, 11000, "Code.exe", "lib.rs", None),
        ];
        let b = build(20000, &events, "", "", &[]);

        assert!(b.just_before.is_none());
    }

    #[test]
    fn test_empty_events() {
        let b = build(50000, &[], "profile", "patterns", &[]);

        assert_eq!(b.right_now.app, "unknown");
        assert!(b.just_before.is_none());
        assert!(b.past_hour.is_empty());
        assert_eq!(b.profile_snippet, "profile");
    }

    #[test]
    fn test_active_event_duration_from_now() {
        // Event within the same session (no daemon_start sentinel, so session_start_ts=0)
        // and within the 5-minute liveness gap → should extrapolate.
        let events = vec![event(1, 10000, "Code.exe", "main.rs", None)];
        let b = build(25000, &events, "", "", &[]);

        assert_eq!(b.right_now.duration_ms, 15000); // 25000 - 10000
    }

    fn sentinel(id: i64, ts: i64, kind: &str) -> EventRow {
        EventRow {
            id,
            ts,
            kind: kind.to_string(),
            app: None,
            title: None,
            duration_ms: None,
            mode: None,
            ocr_text: None,
            vision_desc: None,
        }
    }

    #[test]
    fn test_stale_event_no_session_becomes_unknown() {
        // Daemon was off for hours: last app_focus at ts=1000, now=10_000_000 (way past liveness gap).
        // No daemon_start sentinel → session_start_ts=0, but the gap is > MAX_LIVENESS_GAP_MS.
        let events = vec![event(1, 1000, "Code.exe", "main.rs", None)];
        let b = build(10_000_000, &events, "", "", &[]);

        // Stale NULL-duration event should show "unknown" not "Code.exe"
        assert_eq!(b.right_now.app, "unknown");
        assert_eq!(b.right_now.duration_ms, 0);
    }

    #[test]
    fn test_previous_session_event_not_extrapolated() {
        // daemon_start at ts=50000 marks the session boundary.
        // An app_focus at ts=1000 (before the sentinel) with NULL duration should NOT
        // get extrapolated to now_ms - 1000. The sentinel blocks it.
        let events = vec![
            event(1, 1000, "Code.exe", "main.rs", None),
            sentinel(2, 50000, "daemon_start"),
        ];
        let b = build(55000, &events, "", "", &[]);

        // The app_focus is from before daemon_start → stale
        assert_eq!(b.right_now.app, "unknown");
        assert_eq!(b.right_now.duration_ms, 0);
    }

    #[test]
    fn test_current_session_event_extrapolated() {
        // daemon_start at ts=50000, app_focus at ts=52000 (after sentinel, within liveness gap).
        let events = vec![
            sentinel(1, 50000, "daemon_start"),
            event(2, 52000, "Code.exe", "main.rs", None),
        ];
        let b = build(55000, &events, "", "", &[]);

        assert_eq!(b.right_now.app, "Code.exe");
        assert_eq!(b.right_now.duration_ms, 3000); // 55000 - 52000
    }

    #[test]
    fn test_finalized_event_unaffected_by_session_boundary() {
        // An event from a previous session with a finalized duration_ms should still
        // contribute normally to aggregates — only NULL durations are capped.
        let events = vec![
            event(1, 1000, "Code.exe", "main.rs", Some(30000)),
            sentinel(2, 50000, "daemon_start"),
            event(3, 52000, "chrome.exe", "Google", None),
        ];
        let b = build(55000, &events, "", "", &[]);

        assert_eq!(b.right_now.app, "chrome.exe");
        assert_eq!(b.right_now.duration_ms, 3000);
        // Code.exe should appear in past_hour with its original 30s
        let code_agg = b.past_hour.iter().find(|a| a.app == "Code.exe");
        assert!(code_agg.is_some());
        assert_eq!(code_agg.unwrap().total_ms, 30000);
    }

    #[test]
    fn test_past_hour_aggregate_respects_staleness() {
        // An old NULL-duration event should contribute 0 to aggregates, not hours.
        let events = vec![
            event(1, 1000, "Code.exe", "main.rs", None), // stale
            sentinel(2, 5_000_000, "daemon_start"),
            event(3, 5_001_000, "chrome.exe", "Google", None),
        ];
        let b = build(5_002_000, &events, "", "", &[]);

        // Code.exe aggregate should have 0ms (stale NULL), not millions
        let code_agg = b.past_hour.iter().find(|a| a.app == "Code.exe");
        // Either it's missing entirely (0 duration filtered/aggregated) or total_ms is 0
        if let Some(agg) = code_agg {
            assert_eq!(agg.total_ms, 0);
        }
        // chrome should be 1000ms
        let chrome_agg = b.past_hour.iter().find(|a| a.app == "chrome.exe").unwrap();
        assert_eq!(chrome_agg.total_ms, 1000);
    }

    // ---- BriefingV2 tests ----

    fn url_evt(id: i64, ts: i64, url: &str) -> EventRow {
        EventRow {
            id,
            ts,
            kind: "url".to_string(),
            app: None,
            title: Some(url.to_string()),
            duration_ms: None,
            mode: None,
            ocr_text: None,
            vision_desc: None,
        }
    }

    fn ocr_event(
        id: i64,
        ts: i64,
        app: &str,
        title: &str,
        duration_ms: Option<i64>,
        ocr_text: Option<&str>,
    ) -> EventRow {
        EventRow {
            id,
            ts,
            kind: "app_focus".to_string(),
            app: Some(app.to_string()),
            title: if title.is_empty() {
                None
            } else {
                Some(title.to_string())
            },
            duration_ms,
            mode: None,
            ocr_text: ocr_text.map(|s| s.to_string()),
            vision_desc: None,
        }
    }

    #[test]
    fn test_build_v2_happy_path() {
        let events = vec![
            event(1, 1000, "Code.exe", "main.rs", Some(5000)),
            event(2, 6000, "WindowsTerminal.exe", "PowerShell", Some(7000)),
            event(3, 13000, "Code.exe", "lib.rs", None),
        ];
        let b = build_v2(20000, &events, "my profile", "my patterns", &[]);

        assert_eq!(b.events.len(), 3);
        assert_eq!(b.events[0].app, "Code.exe");
        assert_eq!(b.events[0].duration_ms, 5000);
        assert_eq!(b.events[1].app, "WindowsTerminal.exe");
        assert_eq!(b.events[2].app, "Code.exe");
        // Last event is active: 20000 - 13000 = 7000
        assert_eq!(b.events[2].duration_ms, 7000);
        assert_eq!(b.metrics.switch_count, 3);
        assert!(b.metrics.avg_session_duration_ms > 0);
        assert!(!b.metrics.is_currently_afk);
        assert!(!b.metrics.transitioned_afk_to_active);
        assert_eq!(b.memory.profile, "my profile");
        assert_eq!(b.memory.patterns, "my patterns");
    }

    #[test]
    fn test_build_v2_empty_events() {
        let b = build_v2(50000, &[], "profile", "patterns", &[]);

        assert!(b.events.is_empty());
        assert_eq!(b.metrics.switch_count, 0);
        assert_eq!(b.metrics.avg_session_duration_ms, 0);
        assert!(!b.metrics.is_currently_afk);
        assert!(!b.metrics.transitioned_afk_to_active);
    }

    #[test]
    fn test_build_v2_afk_detection() {
        let events = vec![
            event(1, 1000, "Code.exe", "main.rs", Some(5000)),
            sentinel(2, 6000, "idle_start"),
            event(3, 12000, "chrome.exe", "Google", None),
        ];
        let b = build_v2(20000, &events, "", "", &[]);

        assert!(b.metrics.is_currently_afk);
    }

    #[test]
    fn test_build_v2_afk_transition() {
        let events = vec![
            sentinel(1, 1000, "idle_start"),
            sentinel(2, 5000, "idle_end"),
            event(3, 6000, "Code.exe", "main.rs", None),
        ];
        let b = build_v2(15000, &events, "", "", &[]);

        assert!(!b.metrics.is_currently_afk);
        assert!(b.metrics.transitioned_afk_to_active);
    }

    #[test]
    fn test_build_v2_url_merging() {
        let events = vec![
            event(1, 1000, "Code.exe", "main.rs", Some(5000)),
            url_evt(2, 3000, "https://docs.rs/foo"),
            event(3, 6000, "chrome.exe", "Google", None),
        ];
        let b = build_v2(20000, &events, "", "", &[]);

        // The Code.exe event should not have URL (no URL before it)
        assert!(b.events[0].url.is_none());
        // The chrome.exe event should pick up the URL at ts=3000
        assert_eq!(b.events[1].url.as_deref(), Some("https://docs.rs/foo"));
    }

    #[test]
    fn test_build_v2_ocr_preserved() {
        let events = vec![
            ocr_event(
                1,
                1000,
                "WindowsTerminal.exe",
                "PowerShell",
                Some(8000),
                Some("cargo test\noutput..."),
            ),
            event(2, 9000, "Code.exe", "lib.rs", None),
        ];
        let b = build_v2(20000, &events, "", "", &[]);

        assert_eq!(b.events.len(), 2);
        assert_eq!(
            b.events[0].ocr_text.as_deref(),
            Some("cargo test\noutput...")
        );
        assert!(b.events[1].ocr_text.is_none());
    }

    #[test]
    fn test_build_v2_filter_outside_window() {
        // Event at ts=1000 is more than 5 min before now_ms=500000
        let events = vec![
            event(1, 1000, "Code.exe", "main.rs", Some(5000)),
            event(2, 400_000, "chrome.exe", "Google", None),
        ];
        let b = build_v2(500_000, &events, "", "", &[]);

        // Only the chrome event should be in the 5-min window
        assert_eq!(b.events.len(), 1);
        assert_eq!(b.events[0].app, "chrome.exe");
    }
}
