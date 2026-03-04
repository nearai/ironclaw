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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cleanup_guard_removes_file() {
        let path = "/tmp/ironclaw_cleanup_guard_test.txt";
        std::fs::write(path, "test").unwrap();
        {
            let _guard = CleanupGuard::new().file(path);
            assert!(std::path::Path::new(path).exists());
        }
        assert!(!std::path::Path::new(path).exists());
    }

    #[test]
    fn test_cleanup_guard_removes_dir() {
        let dir = "/tmp/ironclaw_cleanup_guard_test_dir";
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(format!("{dir}/file.txt"), "test").unwrap();
        {
            let _guard = CleanupGuard::new().dir(dir);
            assert!(std::path::Path::new(dir).exists());
        }
        assert!(!std::path::Path::new(dir).exists());
    }
}
