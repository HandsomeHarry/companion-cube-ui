use anyhow::{Context, Result};
use std::path::PathBuf;

/// Resolved data root with standard subdirectories.
pub struct DataRoot {
    pub memory_dir: PathBuf,
    pub data_dir: PathBuf,
    pub logs_dir: PathBuf,
}

impl DataRoot {
    /// Resolve the ccube data root directory.
    ///
    /// Priority: `CCUBE_DATA_DIR` env var > platform default via `directories` crate.
    pub fn resolve() -> Result<Self> {
        let root = if let Ok(custom) = std::env::var("CCUBE_DATA_DIR") {
            PathBuf::from(custom)
        } else {
            let dirs = directories::ProjectDirs::from("", "", "ccube")
                .context("could not determine data directory for this platform")?;
            dirs.data_dir().to_path_buf()
        };

        let memory_dir = root.join("memory");
        let data_dir = root.join("data");
        let logs_dir = root.join("logs");

        std::fs::create_dir_all(&memory_dir)
            .with_context(|| format!("failed to create memory dir: {}", memory_dir.display()))?;
        std::fs::create_dir_all(&data_dir)
            .with_context(|| format!("failed to create data dir: {}", data_dir.display()))?;
        std::fs::create_dir_all(&logs_dir)
            .with_context(|| format!("failed to create logs dir: {}", logs_dir.display()))?;

        Ok(Self {
            memory_dir,
            data_dir,
            logs_dir,
        })
    }
}
