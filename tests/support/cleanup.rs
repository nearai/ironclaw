//! RAII cleanup guard for test directories and files.

/// Removes listed paths when dropped, ensuring cleanup even on panic.
pub struct CleanupGuard {
    paths: Vec<String>,
}

impl CleanupGuard {
    pub fn new() -> Self {
        Self { paths: Vec::new() }
    }

    /// Register a file path for cleanup on drop.
    pub fn file(mut self, path: impl Into<String>) -> Self {
        self.paths.push(path.into());
        self
    }

    /// Register a directory path for cleanup on drop.
    pub fn dir(mut self, path: impl Into<String>) -> Self {
        self.paths.push(path.into());
        self
    }
}

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = std::fs::remove_file(path);
            let _ = std::fs::remove_dir_all(path);
        }
    }
}
