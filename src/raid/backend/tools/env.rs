use std::path::{Path, PathBuf};

use super::path_utils::{clean_path, resolve_read_tool_path, resolve_tool_path};

#[derive(Debug, Clone)]
pub struct ToolEnvironment {
    cwd: PathBuf,
}

impl ToolEnvironment {
    pub fn new() -> Self {
        Self {
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }

    /// Construct an environment rooted at a specific working directory.
    #[allow(dead_code)]
    pub fn with_cwd(cwd: impl Into<PathBuf>) -> Self {
        Self { cwd: cwd.into() }
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    pub fn resolve_path(&self, path: &str) -> PathBuf {
        resolve_tool_path(&self.cwd, path)
    }

    pub fn resolve_read_path(&self, path: &str) -> std::io::Result<PathBuf> {
        resolve_read_tool_path(&self.cwd, path)
    }

    pub fn canonical_path(&self, path: &Path) -> PathBuf {
        std::fs::canonicalize(path).unwrap_or_else(|_| clean_path(path))
    }
}

impl Default for ToolEnvironment {
    fn default() -> Self {
        Self::new()
    }
}
