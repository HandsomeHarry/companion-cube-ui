use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;
use std::process::Command;

use crate::daemon_client;
use crate::paths::DataRoot;

/// Target for commands that work on all memory layers.
#[derive(Debug, Clone, clap::ValueEnum)]
pub enum MemoryTarget {
    Profile,
    Patterns,
    Corrections,
}

/// Target for commands that only work on file-based memory (not corrections DB).
#[derive(Debug, Clone, clap::ValueEnum)]
pub enum EditTarget {
    Profile,
    Patterns,
}

impl EditTarget {
    pub fn filename(&self) -> &str {
        match self {
            EditTarget::Profile => "profile.md",
            EditTarget::Patterns => "patterns.md",
        }
    }
}

fn read_memory_file(memory_dir: &Path, target: &EditTarget) -> Result<String> {
    match target {
        EditTarget::Profile => ccube_core::memory::read_profile(memory_dir),
        EditTarget::Patterns => ccube_core::memory::read_patterns(memory_dir),
    }
}

#[derive(Deserialize)]
struct ProfileResponse {
    content: String,
}

#[derive(Deserialize)]
struct PatternsResponse {
    content: String,
    char_count: usize,
    #[allow(dead_code)]
    updated_at: Option<i64>,
}

pub async fn handle_show(root: &DataRoot, target: &MemoryTarget) -> Result<()> {
    match target {
        MemoryTarget::Profile => {
            // Try daemon HTTP first
            if let Ok(resp) = daemon_client::get_json::<ProfileResponse>("/memory/profile").await {
                if resp.content.is_empty() {
                    println!("No profile yet. Run `ccube memory edit profile` to create one.");
                } else {
                    print!("{}", resp.content);
                }
                return Ok(());
            }

            // Fallback: direct file access
            let content = ccube_core::memory::read_profile(&root.memory_dir)?;
            if content.is_empty() {
                println!("No profile yet. Run `ccube memory edit profile` to create one.");
            } else {
                print!("{content}");
            }
        }
        MemoryTarget::Patterns => {
            // Try daemon HTTP first
            if let Ok(resp) = daemon_client::get_json::<PatternsResponse>("/memory/patterns").await
            {
                if resp.content.is_empty() {
                    println!(
                        "No patterns yet. Patterns are learned from your corrections over time."
                    );
                } else {
                    let hash = ccube_core::memory::patterns_hash(&resp.content);
                    println!(
                        "--- patterns.md ({} chars, hash: {}…) ---",
                        resp.char_count,
                        &hash[..12]
                    );
                    print!("{}", resp.content);
                }
                return Ok(());
            }

            // Fallback: direct file access
            let content = ccube_core::memory::read_patterns(&root.memory_dir)?;
            if content.is_empty() {
                println!("No patterns yet. Patterns are learned from your corrections over time.");
            } else {
                let hash = ccube_core::memory::patterns_hash(&content);
                println!(
                    "--- patterns.md ({} chars, hash: {}…) ---",
                    content.len(),
                    &hash[..12]
                );
                print!("{content}");
            }
        }
        MemoryTarget::Corrections => {
            // Direct access only (no HTTP endpoint for corrections in Phase 3)
            ccube_core::db::init_databases(&root.data_dir)?;
            let conn = ccube_core::db::open_corrections_db(&root.data_dir)?;
            let rows = ccube_core::db::list_corrections(&conn, 20, false)?;
            if rows.is_empty() {
                println!("No corrections recorded yet.");
            } else {
                println!(
                    "{:<5} {:<22} {:<10} {:<30} {:<10}",
                    "ID", "Timestamp", "Original", "Verdict", "Status"
                );
                println!("{}", "-".repeat(77));
                for row in &rows {
                    let dt = chrono::DateTime::from_timestamp_millis(row.ts)
                        .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_else(|| row.ts.to_string());
                    println!(
                        "{:<5} {:<22} {:<10} {:<30} {:<10}",
                        row.id, dt, row.original_decision, row.user_verdict, row.status
                    );
                }
            }
        }
    }
    Ok(())
}

pub fn handle_edit(root: &DataRoot, target: &EditTarget) -> Result<()> {
    let filename = target.filename();
    let original = read_memory_file(&root.memory_dir, target)?;

    // Write current content to a temp file for the editor
    let tmp_path = std::env::temp_dir().join(format!("ccube-edit-{}", filename));
    std::fs::write(&tmp_path, &original)
        .with_context(|| format!("failed to write temp file: {}", tmp_path.display()))?;

    // Resolve editor
    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| "notepad".to_string());

    let status = Command::new(&editor)
        .arg(&tmp_path)
        .status()
        .with_context(|| format!("failed to launch editor: {editor}"))?;

    if !status.success() {
        // Clean up temp file on error
        let _ = std::fs::remove_file(&tmp_path);
        anyhow::bail!("editor exited with status: {status}");
    }

    let new_content = std::fs::read_to_string(&tmp_path)
        .with_context(|| format!("failed to read back temp file: {}", tmp_path.display()))?;
    let _ = std::fs::remove_file(&tmp_path);

    if new_content == original {
        println!("No changes made.");
    } else {
        ccube_core::memory::atomic_write_with_history(
            &root.memory_dir,
            filename,
            &new_content,
            30,
        )?;
        println!("Saved {}.", filename);
    }

    Ok(())
}

pub fn handle_history(root: &DataRoot, target: &EditTarget) -> Result<()> {
    let filename = target.filename();
    let entries = ccube_core::memory::list_history(&root.memory_dir, filename)?;

    if entries.is_empty() {
        println!("No history for {}.", filename);
        return Ok(());
    }

    println!("{:<16} {:<24} Size", "Timestamp", "Date");
    println!("{}", "-".repeat(50));
    for (ts, path) in &entries {
        let dt = chrono::DateTime::from_timestamp(*ts, 0)
            .map(|d| d.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| ts.to_string());
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        println!("{:<16} {:<24} {} bytes", ts, dt, size);
    }

    Ok(())
}

pub fn handle_restore(root: &DataRoot, target: &EditTarget, timestamp: i64) -> Result<()> {
    let filename = target.filename();
    ccube_core::memory::restore_from_history(&root.memory_dir, filename, timestamp)?;

    let dt = chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|d| d.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| timestamp.to_string());
    println!(
        "Restored {} from snapshot {} ({}).",
        filename, timestamp, dt
    );
    Ok(())
}

pub fn handle_diff(root: &DataRoot, target: &EditTarget, ts1: i64, ts2: i64) -> Result<()> {
    let filename = target.filename();
    let diff = ccube_core::memory::diff_snapshots(&root.memory_dir, filename, ts1, ts2)?;

    if diff.is_empty() {
        println!("Snapshots are identical.");
    } else {
        print!("{diff}");
    }
    Ok(())
}
