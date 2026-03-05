//! RAII cleanup guard for test directories and files.

/// The kind of path registered for cleanup.
enum PathKind {
    File,
    Dir,
}

/// Removes listed paths when dropped, ensuring cleanup even on panic.
#[allow(dead_code)]
pub struct CleanupGuard {
    paths: Vec<(String, PathKind)>,
}

#[allow(dead_code)]
impl CleanupGuard {
    pub fn new() -> Self {
        Self { paths: Vec::new() }
    }

    /// Register a file path for cleanup on drop.
    pub fn file(mut self, path: impl Into<String>) -> Self {
        self.paths.push((path.into(), PathKind::File));
        self
    }

    /// Register a directory path for cleanup on drop.
    pub fn dir(mut self, path: impl Into<String>) -> Self {
        self.paths.push((path.into(), PathKind::Dir));
        self
    }
}

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        for (path, kind) in &self.paths {
            match kind {
                PathKind::File => {
                    let _ = std::fs::remove_file(path);
                }
                PathKind::Dir => {
                    let _ = std::fs::remove_dir_all(path);
                }
            }
        }
    }
}
