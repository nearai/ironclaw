//! Shared path validation utilities for tools that access the filesystem.
//!
//! This module provides secure path validation to prevent directory traversal
//! attacks and ensure paths stay within allowed sandboxes.

use std::path::{Path, PathBuf};

use crate::tools::tool::ToolError;

/// Normalize a path by resolving `.` and `..` components lexically (no filesystem access).
///
/// This is critical for security: `std::fs::canonicalize` only works on paths that exist,
/// so for new files we must normalize without touching the filesystem.
pub fn normalize_lexical(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                // Only pop if there's a normal component to pop (don't escape root/prefix)
                if components
                    .last()
                    .is_some_and(|c| matches!(c, std::path::Component::Normal(_)))
                {
                    components.pop();
                }
            }
            std::path::Component::CurDir => {}
            other => components.push(other),
        }
    }
    components.iter().collect()
}

/// Validate that a path is safe (no traversal attacks).
///
/// For sandboxed paths (base_dir is set), we normalize the joined path lexically
/// and then verify it lives under the canonical base. This prevents escapes through
/// non-existent parent directories where `canonicalize()` would fall back to the
/// raw (un-normalized) path.
///
/// # Arguments
/// * `path_str` - The path to validate
/// * `base_dir` - Optional base directory for sandboxing
///
/// # Returns
/// * `Ok(resolved_path)` - The canonicalized, validated path
/// * `Err(ToolError)` - If path escapes sandbox or is invalid
pub fn validate_path(path_str: &str, base_dir: Option<&Path>) -> Result<PathBuf, ToolError> {
    let path = PathBuf::from(path_str);

    // Resolve to absolute path
    let resolved = if path.is_absolute() {
        path.canonicalize()
            .unwrap_or_else(|_| normalize_lexical(&path))
    } else if let Some(base) = base_dir {
        let joined = base.join(&path);
        joined
            .canonicalize()
            .unwrap_or_else(|_| normalize_lexical(&joined))
    } else {
        let joined = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(&path);
        normalize_lexical(&joined)
    };

    // If base_dir is set, ensure the resolved path is within it
    if let Some(base) = base_dir {
        let base_canonical = base
            .canonicalize()
            .unwrap_or_else(|_| normalize_lexical(base));

        // For existing paths, canonicalize to resolve symlinks.
        // For non-existent paths, the lexical normalization above already removed
        // all `..` components, so starts_with is reliable.
        let check_path = if resolved.exists() {
            resolved.canonicalize().unwrap_or_else(|_| resolved.clone())
        } else {
            // Walk up to the nearest existing ancestor directory, canonicalize it,
            // then re-append the remaining tail. This handles the case where a
            // symlink sits above the new file.
            let mut ancestor = resolved.as_path();
            let mut tail_parts: Vec<&std::ffi::OsStr> = Vec::new();
            loop {
                if ancestor.exists() {
                    let canonical_ancestor = ancestor
                        .canonicalize()
                        .unwrap_or_else(|_| ancestor.to_path_buf());
                    let mut result = canonical_ancestor;
                    for part in tail_parts.into_iter().rev() {
                        result = result.join(part);
                    }
                    break result;
                }
                if let Some(name) = ancestor.file_name() {
                    tail_parts.push(name);
                }
                match ancestor.parent() {
                    Some(parent) if parent != ancestor => ancestor = parent,
                    _ => break resolved.clone(),
                }
            }
        };

        if !check_path.starts_with(&base_canonical) {
            return Err(ToolError::NotAuthorized(format!(
                "Path escapes sandbox: {}",
                path_str
            )));
        }
    }

    Ok(resolved)
}

/// Basic path safety check without requiring a base directory.
///
/// This is a fallback check that blocks obvious traversal attempts:
/// - Contains `..` components
/// - Contains null bytes
/// - Uses URL encoding to hide traversal
///
/// For stronger security, use validate_path() with a base_dir.
pub fn is_path_safe_basic(path: &str) -> bool {
    // Block path traversal
    if path.contains("..") {
        return false;
    }

    // Block null bytes (would panic in Path)
    if path.contains('\0') {
        return false;
    }

    // Block URL-encoded traversal attempts
    let lower = path.to_lowercase();
    if lower.contains("%2e") || lower.contains("%2f") || lower.contains("%5c") {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_is_path_safe_basic_allows_normal_paths() {
        assert!(is_path_safe_basic("/tmp/file.txt"));
        assert!(is_path_safe_basic("documents/report.pdf"));
        assert!(is_path_safe_basic("my-file.png"));
    }

    #[test]
    fn test_is_path_safe_basic_rejects_traversal() {
        assert!(!is_path_safe_basic("../etc/passwd"));
        assert!(!is_path_safe_basic("foo/../bar"));
        assert!(!is_path_safe_basic("foo/bar/../../secret"));
    }

    #[test]
    fn test_is_path_safe_basic_rejects_null_bytes() {
        assert!(!is_path_safe_basic("file\0.txt"));
        assert!(!is_path_safe_basic("/tmp/test\0.txt"));
    }

    #[test]
    fn test_is_path_safe_basic_rejects_url_encoding() {
        assert!(!is_path_safe_basic("%2e%2e%2fetc/passwd"));
        assert!(!is_path_safe_basic("foo%2fbar"));
        assert!(!is_path_safe_basic("test%5cpath"));
    }

    #[test]
    fn test_validate_path_allows_within_sandbox() {
        let dir = tempdir().unwrap();
        let result = validate_path("subdir/file.txt", Some(dir.path()));
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_path_rejects_traversal_nonexistent_parent() {
        let dir = tempdir().unwrap();
        // Create a sibling directory structure to test escape
        // Try to escape to parent and access /etc/passwd
        let result = validate_path("../etc/passwd", Some(dir.path()));
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_path_rejects_relative_traversal() {
        let dir = tempdir().unwrap();
        let result = validate_path("../../etc/passwd", Some(dir.path()));
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_path_allows_valid_nested_write() {
        let dir = tempdir().unwrap();
        let result = validate_path("subdir/newfile.txt", Some(dir.path()));
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_path_allows_dot_dot_within_sandbox() {
        let dir = tempdir().unwrap();
        // This should be allowed as it stays within the sandbox
        let result = validate_path("a/b/../c.txt", Some(dir.path()));
        assert!(result.is_ok());
    }
}
