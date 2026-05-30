//! Rhythm analytics — pure computation over captured activity events.
//!
//! No I/O, no LLM calls: every function here is a deterministic transform over a
//! slice of [`EventRow`], so the whole module is unit-testable with fixtures.
//!
//! ## Focus semantics
//! The events table stores a `mode` only on `app_focus` rows, set by
//! [`crate::focus_mode`] to one of `"Coding"`, `"Writing"`, `"VideoProduction"`,
//! or `"Unspecified"`. A row counts as *focus* when it carries a classified
//! mode that is not `"Unspecified"` — this stays correct if new focus
//! categories are added later. `"Unspecified"` and unset modes are treated as
//! non-focus (browsing / drift).

use crate::db::EventRow;
use chrono::{DateTime, Datelike, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RhythmReport {
    pub focus_windows: Vec<FocusWindow>,
    pub fingerprint: Vec<AppCluster>,
    pub drift_origins: Vec<DriftOrigin>,
    pub heatmap: HeatmapData,
}

/// A contiguous time-of-day window ranked by total focus duration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FocusWindow {
    pub hour_start: u8,
    pub hour_end: u8,
    pub total_focus_ms: i64,
    pub label: String,
}

/// A set of apps that recur together during focus sessions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppCluster {
    pub apps: Vec<String>,
    pub session_count: u32,
}

/// Where focus tends to break — the app a user drifts *to* and the focus app
/// they most often drift *from*.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DriftOrigin {
    pub app: String,
    pub from_app: String,
    pub count: u32,
}

/// 24×7 activity grid, row-major: `cells[day * 24 + hour]`, day 0 = Monday.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeatmapData {
    pub cells: Vec<u32>,
    pub max_value: u32,
    pub day_labels: Vec<String>,
    pub hour_labels: Vec<String>,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Compute the full rhythm report from a slice of events.
///
/// `events` may be in any order; functions that need ordering sort internally.
pub fn compute_rhythm(events: &[EventRow]) -> RhythmReport {
    RhythmReport {
        focus_windows: compute_focus_windows(events),
        fingerprint: compute_fingerprint(events),
        drift_origins: compute_drift_origins(events),
        heatmap: compute_heatmap(events),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A row is "focus" when it carries a classified mode other than `Unspecified`.
fn is_focus(mode: Option<&str>) -> bool {
    matches!(mode, Some(m) if !m.is_empty() && m != "Unspecified")
}

fn is_app_focus(e: &EventRow) -> bool {
    e.kind == "app_focus"
}

fn hour_of(ts: i64) -> Option<usize> {
    DateTime::<Utc>::from_timestamp_millis(ts).map(|dt| dt.hour() as usize)
}

/// Format an hour (0–24) as a 12-hour clock label, e.g. `9 AM`, `12 PM`, `12 AM`.
fn format_hour(h: u8) -> String {
    match h {
        0 | 24 => "12 AM".to_string(),
        12 => "12 PM".to_string(),
        1..=11 => format!("{h} AM"),
        _ => format!("{} PM", h - 12),
    }
}

// ---------------------------------------------------------------------------
// Best focus windows
// ---------------------------------------------------------------------------

/// Bin focus duration into 24 hourly buckets, then greedily pick up to three
/// non-overlapping 2–4h windows with the highest combined focus time.
fn compute_focus_windows(events: &[EventRow]) -> Vec<FocusWindow> {
    let mut buckets = [0i64; 24];
    for e in events {
        if is_app_focus(e) && is_focus(e.mode.as_deref()) {
            if let Some(h) = hour_of(e.ts) {
                buckets[h] += e.duration_ms.unwrap_or(0);
            }
        }
    }

    // Candidate windows of length 2..=4 hours.
    let mut candidates: Vec<FocusWindow> = Vec::new();
    for len in 2..=4usize {
        for start in 0..=(24 - len) {
            let total: i64 = buckets[start..start + len].iter().sum();
            if total > 0 {
                let end = start + len;
                candidates.push(FocusWindow {
                    hour_start: start as u8,
                    hour_end: end as u8,
                    total_focus_ms: total,
                    label: format!("{} – {}", format_hour(start as u8), format_hour(end as u8)),
                });
            }
        }
    }

    // Highest total first; tie-break on the shorter (denser) window.
    candidates.sort_by(|a, b| {
        b.total_focus_ms
            .cmp(&a.total_focus_ms)
            .then((a.hour_end - a.hour_start).cmp(&(b.hour_end - b.hour_start)))
    });

    // Greedily select non-overlapping windows.
    let mut chosen: Vec<FocusWindow> = Vec::new();
    for cand in candidates {
        let overlaps = chosen
            .iter()
            .any(|c| cand.hour_start < c.hour_end && c.hour_start < cand.hour_end);
        if !overlaps {
            chosen.push(cand);
            if chosen.len() == 3 {
                break;
            }
        }
    }
    chosen
}

// ---------------------------------------------------------------------------
// Focus fingerprint
// ---------------------------------------------------------------------------

/// Group focus events into 2-hour windows and count which distinct app-sets
/// recur. Returns the top 5 multi-app combinations by occurrence.
fn compute_fingerprint(events: &[EventRow]) -> Vec<AppCluster> {
    let mut focus: Vec<&EventRow> = events
        .iter()
        .filter(|e| is_app_focus(e) && is_focus(e.mode.as_deref()) && e.app.is_some())
        .collect();
    focus.sort_by_key(|e| e.ts);

    if focus.is_empty() {
        return Vec::new();
    }

    const WINDOW_MS: i64 = 2 * 60 * 60 * 1000;
    let mut counts: HashMap<Vec<String>, u32> = HashMap::new();
    let mut window_start = focus[0].ts;
    let mut apps: Vec<String> = Vec::new();

    let flush = |apps: &mut Vec<String>, counts: &mut HashMap<Vec<String>, u32>| {
        apps.sort();
        apps.dedup();
        if apps.len() >= 2 {
            *counts.entry(std::mem::take(apps)).or_insert(0) += 1;
        } else {
            apps.clear();
        }
    };

    for e in &focus {
        if e.ts - window_start > WINDOW_MS {
            flush(&mut apps, &mut counts);
            window_start = e.ts;
        }
        if let Some(app) = &e.app {
            apps.push(app.clone());
        }
    }
    flush(&mut apps, &mut counts);

    let mut clusters: Vec<AppCluster> = counts
        .into_iter()
        .map(|(apps, session_count)| AppCluster { apps, session_count })
        .collect();
    // Most frequent first; tie-break on richer combos then lexical for determinism.
    clusters.sort_by(|a, b| {
        b.session_count
            .cmp(&a.session_count)
            .then(b.apps.len().cmp(&a.apps.len()))
            .then(a.apps.cmp(&b.apps))
    });
    clusters.truncate(5);
    clusters
}

// ---------------------------------------------------------------------------
// Drift origins
// ---------------------------------------------------------------------------

/// Detect focus→non-focus app switches within 5 minutes and aggregate by the
/// app drifted *to*, recording the focus app most often left behind.
fn compute_drift_origins(events: &[EventRow]) -> Vec<DriftOrigin> {
    let mut focus_changes: Vec<&EventRow> = events.iter().filter(|e| is_app_focus(e)).collect();
    focus_changes.sort_by_key(|e| e.ts);

    const GAP_MS: i64 = 5 * 60 * 1000;
    // to_app -> (from_app -> count)
    let mut transitions: HashMap<String, HashMap<String, u32>> = HashMap::new();

    for pair in focus_changes.windows(2) {
        let (prev, curr) = (pair[0], pair[1]);
        if curr.ts - prev.ts > GAP_MS {
            continue;
        }
        if !is_focus(prev.mode.as_deref()) || is_focus(curr.mode.as_deref()) {
            continue;
        }
        let (Some(from), Some(to)) = (prev.app.as_ref(), curr.app.as_ref()) else {
            continue;
        };
        *transitions
            .entry(to.clone())
            .or_default()
            .entry(from.clone())
            .or_insert(0) += 1;
    }

    let mut origins: Vec<DriftOrigin> = transitions
        .into_iter()
        .map(|(app, froms)| {
            let count = froms.values().sum();
            // Most common source app; lexical tie-break for determinism.
            let from_app = froms
                .into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
                .map(|(f, _)| f)
                .unwrap_or_default();
            DriftOrigin { app, from_app, count }
        })
        .collect();

    origins.sort_by(|a, b| b.count.cmp(&a.count).then(a.app.cmp(&b.app)));
    origins.truncate(5);
    origins
}

// ---------------------------------------------------------------------------
// Heatmap
// ---------------------------------------------------------------------------

/// Aggregate active app time into a 24×7 grid (minutes), row-major with day 0 =
/// Monday.
fn compute_heatmap(events: &[EventRow]) -> HeatmapData {
    let mut cells = vec![0u32; 168];
    for e in events {
        if !is_app_focus(e) {
            continue;
        }
        let Some(ms) = e.duration_ms else { continue };
        if ms <= 0 {
            continue;
        }
        if let Some(dt) = DateTime::<Utc>::from_timestamp_millis(e.ts) {
            let day = dt.weekday().num_days_from_monday() as usize; // Mon = 0
            let hour = dt.hour() as usize;
            let idx = day * 24 + hour;
            if idx < 168 {
                cells[idx] = cells[idx].saturating_add((ms / 60_000) as u32);
            }
        }
    }
    let max_value = cells.iter().copied().max().unwrap_or(0).max(1);
    HeatmapData {
        cells,
        max_value,
        day_labels: ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        hour_labels: (0..24).map(|h| format_hour(h as u8)).collect(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an `app_focus` event at a UTC wall-clock time.
    fn ev(id: i64, y: i32, mo: u32, d: u32, h: u32, min: u32, app: &str, mode: Option<&str>, dur_ms: Option<i64>) -> EventRow {
        let ts = chrono::NaiveDate::from_ymd_opt(y, mo, d)
            .unwrap()
            .and_hms_opt(h, min, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis();
        EventRow {
            id,
            ts,
            kind: "app_focus".to_string(),
            app: Some(app.to_string()),
            title: None,
            duration_ms: dur_ms,
            mode: mode.map(|m| m.to_string()),
            ocr_text: None,
            vision_desc: None,
        }
    }

    #[test]
    fn empty_input_is_safe() {
        let r = compute_rhythm(&[]);
        assert!(r.focus_windows.is_empty());
        assert!(r.fingerprint.is_empty());
        assert!(r.drift_origins.is_empty());
        assert_eq!(r.heatmap.cells.len(), 168);
        assert_eq!(r.heatmap.max_value, 1, "max clamps to 1 to avoid div-by-zero");
        assert_eq!(r.heatmap.day_labels.len(), 7);
        assert_eq!(r.heatmap.hour_labels.len(), 24);
    }

    #[test]
    fn unspecified_mode_is_not_focus() {
        // 2026-05-25 is a Monday. One hour of "Coding", one of "Unspecified".
        let events = vec![
            ev(1, 2026, 5, 25, 9, 0, "VS Code", Some("Coding"), Some(60 * 60 * 1000)),
            ev(2, 2026, 5, 25, 10, 0, "Safari", Some("Unspecified"), Some(60 * 60 * 1000)),
        ];
        let fw = compute_focus_windows(&events);
        // Only the 9:00 hour carries focus time. The chosen window must cover
        // hour 9 and credit exactly one focus hour — the 10:00 "Unspecified"
        // hour contributes nothing (windows 8–10 and 9–11 tie; either is valid).
        assert!(!fw.is_empty());
        assert!(fw[0].hour_start <= 9 && fw[0].hour_end > 9, "window must cover hour 9");
        assert_eq!(fw[0].total_focus_ms, 60 * 60 * 1000, "only one focus hour counts");
    }

    #[test]
    fn focus_windows_pick_non_overlapping_peaks() {
        // Strong morning block (9–11) and a separate afternoon block (14–16).
        let events = vec![
            ev(1, 2026, 5, 25, 9, 0, "VS Code", Some("Coding"), Some(60 * 60 * 1000)),
            ev(2, 2026, 5, 25, 10, 0, "VS Code", Some("Coding"), Some(60 * 60 * 1000)),
            ev(3, 2026, 5, 25, 14, 0, "Xcode", Some("Coding"), Some(30 * 60 * 1000)),
            ev(4, 2026, 5, 25, 15, 0, "Xcode", Some("Coding"), Some(30 * 60 * 1000)),
        ];
        let fw = compute_focus_windows(&events);
        assert_eq!(fw[0].hour_start, 9, "morning is the strongest block");
        // The second window must not overlap the first.
        assert!(fw.len() >= 2);
        assert!(fw[1].hour_start >= fw[0].hour_end || fw[1].hour_end <= fw[0].hour_start);
        assert_eq!(fw[1].hour_start, 14);
    }

    #[test]
    fn fingerprint_counts_recurring_app_sets() {
        // Two separate 2h windows, each pairing VS Code + Terminal.
        let events = vec![
            ev(1, 2026, 5, 25, 9, 0, "VS Code", Some("Coding"), Some(30 * 60 * 1000)),
            ev(2, 2026, 5, 25, 9, 30, "Terminal", Some("Coding"), Some(10 * 60 * 1000)),
            ev(3, 2026, 5, 25, 13, 0, "VS Code", Some("Coding"), Some(30 * 60 * 1000)),
            ev(4, 2026, 5, 25, 13, 30, "Terminal", Some("Coding"), Some(10 * 60 * 1000)),
        ];
        let fp = compute_fingerprint(&events);
        assert!(!fp.is_empty());
        assert_eq!(fp[0].apps, vec!["Terminal".to_string(), "VS Code".to_string()]);
        assert_eq!(fp[0].session_count, 2);
    }

    #[test]
    fn fingerprint_ignores_single_app_windows() {
        let events = vec![
            ev(1, 2026, 5, 25, 9, 0, "VS Code", Some("Coding"), Some(30 * 60 * 1000)),
            ev(2, 2026, 5, 25, 9, 30, "VS Code", Some("Coding"), Some(30 * 60 * 1000)),
        ];
        assert!(compute_fingerprint(&events).is_empty());
    }

    #[test]
    fn drift_origins_track_focus_to_nonfocus() {
        // Coding -> YouTube(Unspecified) twice, within 5 min each.
        let events = vec![
            ev(1, 2026, 5, 25, 10, 0, "VS Code", Some("Coding"), Some(60 * 1000)),
            ev(2, 2026, 5, 25, 10, 1, "YouTube", Some("Unspecified"), Some(60 * 1000)),
            ev(3, 2026, 5, 25, 10, 3, "VS Code", Some("Coding"), Some(60 * 1000)),
            ev(4, 2026, 5, 25, 10, 4, "YouTube", Some("Unspecified"), Some(60 * 1000)),
        ];
        let d = compute_drift_origins(&events);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].app, "YouTube");
        assert_eq!(d[0].from_app, "VS Code");
        assert_eq!(d[0].count, 2);
    }

    #[test]
    fn drift_ignores_gaps_over_five_minutes() {
        let events = vec![
            ev(1, 2026, 5, 25, 10, 0, "VS Code", Some("Coding"), Some(60 * 1000)),
            ev(2, 2026, 5, 25, 10, 10, "YouTube", Some("Unspecified"), Some(60 * 1000)),
        ];
        assert!(compute_drift_origins(&events).is_empty(), "10-min gap is not a drift");
    }

    #[test]
    fn heatmap_places_minutes_in_monday_nine() {
        // Monday 2026-05-25, 09:xx, 30 minutes.
        let events = vec![ev(1, 2026, 5, 25, 9, 0, "VS Code", Some("Coding"), Some(30 * 60 * 1000))];
        let r = compute_rhythm(&events);
        assert_eq!(r.heatmap.cells[9], 30, "Mon(0)*24 + 9 = cell 9");
        assert_eq!(r.heatmap.max_value, 30);
    }

    #[test]
    fn heatmap_sunday_index() {
        // 2026-05-31 is a Sunday.
        let events = vec![ev(1, 2026, 5, 31, 23, 0, "VLC", Some("Unspecified"), Some(15 * 60 * 1000))];
        let r = compute_rhythm(&events);
        // Sun = day 6, hour 23 -> 6*24 + 23 = 167
        assert_eq!(r.heatmap.cells[167], 15);
    }

    #[test]
    fn non_app_focus_events_excluded() {
        // idle_start / daemon_start carry no mode and must not affect analytics.
        let mut idle = ev(1, 2026, 5, 25, 9, 0, "", None, Some(60 * 60 * 1000));
        idle.kind = "idle_start".to_string();
        idle.app = None;
        let r = compute_rhythm(&[idle]);
        assert!(r.focus_windows.is_empty());
        assert_eq!(r.heatmap.max_value, 1);
    }
}
