use crate::config::DownloadState;
use anyhow::Result;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Manages download state persistence for resume capability
pub struct StateManager {
    state_path: PathBuf,
}

impl StateManager {
    pub fn new(output_path: &Path) -> Self {
        let state_path = output_path.with_extension("nget");
        Self { state_path }
    }

    /// Load download state from disk
    pub fn load_state(&self) -> Result<Option<DownloadState>> {
        if !self.state_path.exists() {
            return Ok(None);
        }

        let mut file = File::open(&self.state_path)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;

        let state: DownloadState = serde_json::from_str(&contents)?;
        Ok(Some(state))
    }

    /// Save download state to disk
    pub fn save_state(&self, state: &DownloadState) -> Result<()> {
        let serialized = serde_json::to_string_pretty(state)?;
        let mut file = File::create(&self.state_path)?;
        file.write_all(serialized.as_bytes())?;
        Ok(())
    }

    /// Clean up state file after successful download
    pub fn cleanup(&self) -> Result<()> {
        if self.state_path.exists() {
            fs::remove_file(&self.state_path)?;
        }
        Ok(())
    }

    /// Check if state file exists
    pub fn state_exists(&self) -> bool {
        self.state_path.exists()
    }
}

impl Clone for StateManager {
    fn clone(&self) -> Self {
        Self {
            state_path: self.state_path.clone(),
        }
    }
}
