use anyhow::Result;
use similar::TextDiff;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing;

/// Read `profile.md` from the memory directory.
pub fn read_profile(memory_dir: &Path) -> Result<String> {
    let path = memory_dir.join("profile.md");
    if path.exists() {
        Ok(std::fs::read_to_string(&path)?)
    } else {
        Ok(String::new())
    }
}

/// Read `patterns.md` from the memory directory.
pub fn read_patterns(memory_dir: &Path) -> Result<String> {
    let path = memory_dir.join("patterns.md");
    if path.exists() {
        Ok(std::fs::read_to_string(&path)?)
    } else {
        Ok(String::new())
    }
}

/// Compute SHA-256 hash of `patterns.md` content.
pub fn patterns_hash(content: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Atomically write content to a memory file, backing up to history.
pub fn atomic_write_with_history(
    memory_dir: &Path,
    filename: &str,
    content: &str,
    max_history: usize,
) -> Result<()> {
    let target = memory_dir.join(filename);
    let history_dir = memory_dir.join(format!("{}.history", filename));
    std::fs::create_dir_all(&history_dir)?;

    // Backup current file if it exists
    if target.exists() {
        let ts = chrono::Utc::now().timestamp_millis();
        let backup = history_dir.join(format!("{}", ts));
        std::fs::copy(&target, &backup)?;
        tracing::debug!("backed up {} to {}", filename, backup.display());
    }

    // Write new content atomically via temp file
    let tmp = target.with_extension("tmp");
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, &target)?;

    // Rotate history: keep only the last `max_history` entries
    rotate_history(&history_dir, max_history)?;

    Ok(())
}

/// List history snapshots for a memory file, sorted newest first.
pub fn list_history(memory_dir: &Path, filename: &str) -> Result<Vec<(i64, PathBuf)>> {
    let history_dir = memory_dir.join(format!("{}.history", filename));
    if !history_dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries: Vec<(i64, PathBuf)> = std::fs::read_dir(&history_dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let ts: i64 = name.parse().ok()?;
            Some((ts, e.path()))
        })
        .collect();

    entries.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(entries)
}

/// Restore a memory file from a history snapshot.
pub fn restore_from_history(memory_dir: &Path, filename: &str, timestamp: i64) -> Result<()> {
    let history_dir = memory_dir.join(format!("{}.history", filename));
    let snapshot = history_dir.join(format!("{}", timestamp));

    if !snapshot.exists() {
        anyhow::bail!("snapshot {} not found for {}", timestamp, filename);
    }

    let content = std::fs::read_to_string(&snapshot)?;
    atomic_write_with_history(memory_dir, filename, &content, 30)?;
    Ok(())
}

fn rotate_history(history_dir: &Path, max: usize) -> Result<()> {
    let mut entries: Vec<(i64, PathBuf)> = std::fs::read_dir(history_dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let ts: i64 = name.parse().ok()?;
            Some((ts, e.path()))
        })
        .collect();

    entries.sort_by(|a, b| b.0.cmp(&a.0));

    for entry in entries.iter().skip(max) {
        std::fs::remove_file(&entry.1)?;
    }

    Ok(())
}

/// Read a specific history snapshot by timestamp.
pub fn read_snapshot(memory_dir: &Path, filename: &str, timestamp: i64) -> Result<String> {
    let history_dir = memory_dir.join(format!("{}.history", filename));
    let snapshot = history_dir.join(format!("{}", timestamp));

    if !snapshot.exists() {
        anyhow::bail!("snapshot {} not found for {}", timestamp, filename);
    }

    Ok(std::fs::read_to_string(&snapshot)?)
}

/// Produce a unified diff between two history snapshots.
pub fn diff_snapshots(memory_dir: &Path, filename: &str, ts1: i64, ts2: i64) -> Result<String> {
    let content1 = read_snapshot(memory_dir, filename, ts1)?;
    let content2 = read_snapshot(memory_dir, filename, ts2)?;

    let diff = TextDiff::from_lines(&content1, &content2);

    let dt1 = chrono::DateTime::from_timestamp_millis(ts1)
        .map(|d| d.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| ts1.to_string());
    let dt2 = chrono::DateTime::from_timestamp_millis(ts2)
        .map(|d| d.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| ts2.to_string());

    let header_old = format!("{} ({})", filename, dt1);
    let header_new = format!("{} ({})", filename, dt2);

    Ok(diff
        .unified_diff()
        .header(&header_old, &header_new)
        .to_string())
}

/// Build a cache mapping patterns_hash -> file content for all history snapshots
/// of `patterns.md`, plus the current live file. Used for context fencing in the
/// curator: each correction sees patterns as they existed at correction time.
pub fn build_patterns_hash_cache(memory_dir: &Path) -> Result<HashMap<String, String>> {
    let mut cache = HashMap::new();

    // Include current patterns.md
    let current = read_patterns(memory_dir)?;
    if !current.is_empty() {
        let hash = patterns_hash(&current);
        cache.insert(hash, current);
    }

    // Include all history snapshots
    let history = list_history(memory_dir, "patterns.md")?;
    for (ts, _path) in &history {
        match read_snapshot(memory_dir, "patterns.md", *ts) {
            Ok(content) => {
                let hash = patterns_hash(&content);
                cache.insert(hash, content);
            }
            Err(e) => {
                tracing::warn!(ts, error = %e, "failed to read patterns history snapshot");
            }
        }
    }

    Ok(cache)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;
    use tempfile::TempDir;

    #[test]
    fn test_read_missing_returns_empty() {
        let dir = TempDir::new().unwrap();
        assert_eq!(read_profile(dir.path()).unwrap(), "");
        assert_eq!(read_patterns(dir.path()).unwrap(), "");
    }

    #[test]
    fn test_write_and_read_roundtrip() {
        let dir = TempDir::new().unwrap();
        let content = "Hello, profile!\nLine 2.";
        atomic_write_with_history(dir.path(), "profile.md", content, 30).unwrap();
        assert_eq!(read_profile(dir.path()).unwrap(), content);
    }

    #[test]
    fn test_write_creates_history() {
        let dir = TempDir::new().unwrap();
        atomic_write_with_history(dir.path(), "profile.md", "v1", 30).unwrap();
        thread::sleep(Duration::from_secs(1));
        atomic_write_with_history(dir.path(), "profile.md", "v2", 30).unwrap();

        let history = list_history(dir.path(), "profile.md").unwrap();
        assert_eq!(history.len(), 1); // first write has no prior file to back up
    }

    #[test]
    fn test_two_edits_produce_two_history_entries() {
        let dir = TempDir::new().unwrap();
        atomic_write_with_history(dir.path(), "profile.md", "v1", 30).unwrap();
        thread::sleep(Duration::from_secs(1));
        atomic_write_with_history(dir.path(), "profile.md", "v2", 30).unwrap();
        thread::sleep(Duration::from_secs(1));
        atomic_write_with_history(dir.path(), "profile.md", "v3", 30).unwrap();

        let history = list_history(dir.path(), "profile.md").unwrap();
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn test_history_rotation() {
        let dir = TempDir::new().unwrap();
        let max = 3;

        atomic_write_with_history(dir.path(), "patterns.md", "v0", max).unwrap();
        for i in 1..=5 {
            thread::sleep(Duration::from_secs(1));
            atomic_write_with_history(dir.path(), "patterns.md", &format!("v{}", i), max).unwrap();
        }

        let history = list_history(dir.path(), "patterns.md").unwrap();
        assert!(
            history.len() <= max,
            "history should be at most {max}, got {}",
            history.len()
        );
    }

    #[test]
    fn test_restore_reverts_content() {
        let dir = TempDir::new().unwrap();
        atomic_write_with_history(dir.path(), "profile.md", "original", 30).unwrap();
        thread::sleep(Duration::from_secs(1));
        atomic_write_with_history(dir.path(), "profile.md", "modified", 30).unwrap();

        let history = list_history(dir.path(), "profile.md").unwrap();
        assert!(!history.is_empty());
        let old_ts = history[0].0;

        restore_from_history(dir.path(), "profile.md", old_ts).unwrap();
        assert_eq!(read_profile(dir.path()).unwrap(), "original");
    }

    #[test]
    fn test_restore_nonexistent_fails() {
        let dir = TempDir::new().unwrap();
        let result = restore_from_history(dir.path(), "profile.md", 9999999999);
        assert!(result.is_err());
    }

    #[test]
    fn test_hash_deterministic() {
        let h1 = patterns_hash("same content");
        let h2 = patterns_hash("same content");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_hash_differs() {
        let h1 = patterns_hash("content A");
        let h2 = patterns_hash("content B");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_read_snapshot() {
        let dir = TempDir::new().unwrap();
        atomic_write_with_history(dir.path(), "patterns.md", "first", 30).unwrap();
        thread::sleep(Duration::from_secs(1));
        atomic_write_with_history(dir.path(), "patterns.md", "second", 30).unwrap();

        let history = list_history(dir.path(), "patterns.md").unwrap();
        let ts = history[0].0;
        let content = read_snapshot(dir.path(), "patterns.md", ts).unwrap();
        assert_eq!(content, "first");
    }

    #[test]
    fn test_diff_identical_empty() {
        let dir = TempDir::new().unwrap();
        atomic_write_with_history(dir.path(), "patterns.md", "same", 30).unwrap();
        thread::sleep(Duration::from_secs(1));
        atomic_write_with_history(dir.path(), "patterns.md", "changed", 30).unwrap();

        let history = list_history(dir.path(), "patterns.md").unwrap();
        let ts = history[0].0;
        // Diff a snapshot against itself — should produce empty output
        let diff = diff_snapshots(dir.path(), "patterns.md", ts, ts).unwrap();
        assert!(
            diff.is_empty(),
            "diff of same snapshot should be empty, got: {}",
            diff
        );
    }

    #[test]
    fn test_diff_shows_changes() {
        let dir = TempDir::new().unwrap();
        atomic_write_with_history(dir.path(), "patterns.md", "line one\n", 30).unwrap();
        thread::sleep(Duration::from_secs(1));
        atomic_write_with_history(dir.path(), "patterns.md", "line two\n", 30).unwrap();
        thread::sleep(Duration::from_secs(1));
        atomic_write_with_history(dir.path(), "patterns.md", "final\n", 30).unwrap();

        let history = list_history(dir.path(), "patterns.md").unwrap();
        let diff = diff_snapshots(dir.path(), "patterns.md", history[1].0, history[0].0).unwrap();

        assert!(
            diff.contains("-line one"),
            "diff should show removed line, got: {}",
            diff
        );
        assert!(
            diff.contains("+line two"),
            "diff should show added line, got: {}",
            diff
        );
    }

    // -----------------------------------------------------------------------
    // Phase 6: context fencing hash cache
    // -----------------------------------------------------------------------

    #[test]
    fn test_hash_cache_finds_historical_version() {
        let dir = TempDir::new().unwrap();
        let v1 = "§ social media is always drift";
        let v2 = "§ social media is always drift\n§ YouTube music is on-task";

        // Write v1, then v2 (v1 goes to history)
        atomic_write_with_history(dir.path(), "patterns.md", v1, 30).unwrap();
        thread::sleep(Duration::from_secs(1));
        atomic_write_with_history(dir.path(), "patterns.md", v2, 30).unwrap();

        let v1_hash = patterns_hash(v1);
        let v2_hash = patterns_hash(v2);

        let cache = build_patterns_hash_cache(dir.path()).unwrap();

        // Both versions should be in cache
        assert_eq!(cache.get(&v1_hash).unwrap(), v1);
        assert_eq!(cache.get(&v2_hash).unwrap(), v2);
    }

    #[test]
    fn test_hash_cache_returns_empty_for_pre_history() {
        let dir = TempDir::new().unwrap();
        atomic_write_with_history(dir.path(), "patterns.md", "some content", 30).unwrap();

        let cache = build_patterns_hash_cache(dir.path()).unwrap();

        // A hash that was never written should not be found
        let unknown_hash = patterns_hash("content that never existed");
        let result = cache.get(&unknown_hash).cloned().unwrap_or_default();
        assert_eq!(result, "");
    }

    #[test]
    fn test_hash_cache_empty_dir() {
        let dir = TempDir::new().unwrap();
        let cache = build_patterns_hash_cache(dir.path()).unwrap();
        assert!(cache.is_empty());
    }
}
