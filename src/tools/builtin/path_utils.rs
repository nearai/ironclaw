//! Shared path validation utilities for tools that access the filesystem.
//!
//! This module provides secure path validation to prevent directory traversal
//! attacks and ensure paths stay within allowed sandboxes.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use crate::tools::tool::ToolError;

/// Paths that contain credentials, secrets, or private keys.
/// Used by both file tools (exact path check) and shell tool (substring scan).
/// Keep sorted by category for readability.
static SENSITIVE_PATH_PATTERNS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    vec![
        // SSH
        "/.ssh/",
        "/id_rsa",
        "/id_ed25519",
        "/id_ecdsa",
        "/id_dsa",
        "/authorized_keys",
        "/known_hosts",
        // GPG
        "/.gnupg/",
        // AWS
        "/.aws/credentials",
        "/.aws/config",
        // Kubernetes
        "/.kube/config",
        // Cloud providers
        "/.azure/",
        "/.gcloud/",
        "/.config/gcloud/",
        // Terraform
        "/.terraform.d/credentials.tfrc.json",
        // GitHub CLI
        "/.config/gh/hosts.yml",
        // Docker
        "/.docker/config.json",
        // Vault
        "/.vault-token",
        // Shell history
        "/.bash_history",
        "/.zsh_history",
        "/.histfile",
        // Env files (may contain secrets)
        "/.env",
        // Git credentials
        "/.git-credentials",
        "/.netrc",
        "/.pgpass",
        // IronClaw's own secrets
        "/.ironclaw/secrets/",
        // System
        "/etc/shadow",
        "/etc/gshadow",
    ]
});

/// File extensions that are always sensitive regardless of location.
static SENSITIVE_EXTENSIONS: LazyLock<Vec<&'static str>> =
    LazyLock::new(|| vec![".pem", ".key", ".p12", ".pfx", ".jks", ".keystore"]);

/// Suffixes that indicate a file is safe despite matching a sensitive pattern
/// (e.g., `.env.example`, `.env.sample`).
static SAFE_SUFFIXES: LazyLock<Vec<&'static str>> =
    LazyLock::new(|| vec![".example", ".sample", ".template", ".dist", ".bak.example"]);

/// Check if a resolved file path points to a sensitive location.
/// Used by file tools (read, write, list_dir, apply_patch).
pub fn is_sensitive_path(path: &Path) -> bool {
    let path_str = match path.canonicalize() {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(_) => path.to_string_lossy().to_string(),
    };

    // Safe suffixes override sensitive patterns
    let lower = path_str.to_lowercase();
    if SAFE_SUFFIXES.iter().any(|s| lower.ends_with(s)) {
        return false;
    }

    // Check sensitive path patterns
    if SENSITIVE_PATH_PATTERNS
        .iter()
        .any(|p| path_pattern_matches(&lower, p))
    {
        return true;
    }

    // Check sensitive file extensions
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let dot_ext = format!(".{}", ext.to_lowercase());
        if SENSITIVE_EXTENSIONS.iter().any(|e| *e == dot_ext) {
            return true;
        }
    }

    false
}

/// Scan a shell command string for references to sensitive paths.
/// Returns the first matched pattern, or None if the command is clean.
/// Used by the shell tool to block `cat ~/.ssh/id_rsa` etc.
///
/// # Limitations
///
/// This is a **defense-in-depth** heuristic operating on a raw command string.
/// It inherently cannot catch every bypass, including but not limited to:
///
/// - **Symlink-based bypasses** (`ln -s ~/.ssh/id_rsa /tmp/link && cat /tmp/link`)
/// - **Encoding tricks** (backtick/subshell expansion, base64-encoded paths, hex escapes)
/// - **Quoted or escaped path segments** that defeat simple token splitting
/// - **Variable expansion** (`$HOME/.ssh/id_rsa`, `${SECRET_FILE}`)
/// - **Glob-based access** (`cat /etc/sh*dow`)
///
/// The **sandbox / Docker isolation layer** is the real security boundary.
/// This scanner exists to catch the common, obvious cases at the tool layer.
pub fn command_references_sensitive_path(command: &str) -> Option<&'static str> {
    let normalized = command.to_lowercase();

    for pattern in SENSITIVE_PATH_PATTERNS.iter() {
        let pat_lower = pattern.to_lowercase();
        if pattern_matches_in_command(&normalized, &pat_lower) {
            return Some(pattern);
        }
    }

    // Check for sensitive extensions in whitespace-delimited tokens.
    // Requiring the extension at the end of a token avoids false positives
    // like `--key-file /tmp/foo` matching `.key`.
    let tokens: Vec<&str> = normalized.split_whitespace().collect();
    SENSITIVE_EXTENSIONS
        .iter()
        .find(|ext| tokens.iter().any(|tok| tok.ends_with(*ext)))
        .copied()
}

/// Check whether a sensitive path pattern matches within a file path string.
/// Handles the `/.env` special case the same way as the command scanner.
fn path_pattern_matches(path_lower: &str, pattern: &str) -> bool {
    if pattern == "/.env" {
        pattern_matches_in_command(path_lower, &pattern.to_lowercase())
    } else {
        path_lower.contains(&pattern.to_lowercase())
    }
}

/// Check whether `pattern` matches in `command` with awareness of the `/.env`
/// special case: `/.env` must match exactly as a complete filename or be
/// followed by `.` (e.g. `/.env.local`), but NOT match `.envrc`,
/// `.environment`, or other extensions that merely start with `env`.
fn pattern_matches_in_command(command: &str, pattern: &str) -> bool {
    if pattern == "/.env" {
        // Find all occurrences of "/.env" and check what follows
        let mut start = 0;
        while let Some(pos) = command[start..].find(pattern) {
            let abs_pos = start + pos;
            let after = abs_pos + pattern.len();
            if after >= command.len() {
                // "/.env" at end of string -- exact match
                return true;
            }
            let next_char = command.as_bytes()[after];
            // Allow /.env followed by: whitespace, '.' (for .env.local etc.), '/' or end
            if next_char == b' '
                || next_char == b'\t'
                || next_char == b'.'
                || next_char == b'/'
                || next_char == b'"'
                || next_char == b'\''
            {
                return true;
            }
            // Otherwise it's something like .envrc -- skip and keep looking
            start = after;
        }
        false
    } else {
        command.contains(pattern)
    }
}

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
    // First pass: reject null bytes and URL-encoded traversal
    // Note: We don't block `..` here because validate_path handles it by
    // normalizing lexically and checking sandbox containment
    if !is_path_safe_minimal(path_str) {
        return Err(ToolError::NotAuthorized(format!(
            "Path contains forbidden characters or sequences: {}",
            path_str
        )));
    }

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

/// Check for null bytes and URL-encoded traversal only.
/// Unlike is_path_safe_basic, this allows `..` in paths since validate_path
/// handles that by normalizing lexically and checking sandbox containment.
fn is_path_safe_minimal(path: &str) -> bool {
    if path.contains('\0') {
        return false;
    }

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

    // ── sensitive path tests ──

    #[test]
    fn test_is_sensitive_path_blocks_ssh() {
        assert!(is_sensitive_path(Path::new("/home/user/.ssh/id_rsa")));
        assert!(is_sensitive_path(Path::new(
            "/home/user/.ssh/authorized_keys"
        )));
        assert!(is_sensitive_path(Path::new("/root/.ssh/config")));
    }

    #[test]
    fn test_is_sensitive_path_blocks_cloud_credentials() {
        assert!(is_sensitive_path(Path::new("/home/user/.aws/credentials")));
        assert!(is_sensitive_path(Path::new("/home/user/.kube/config")));
        assert!(is_sensitive_path(Path::new("/home/user/.azure/some_token")));
        assert!(is_sensitive_path(Path::new(
            "/home/user/.config/gh/hosts.yml"
        )));
    }

    #[test]
    fn test_is_sensitive_path_blocks_system_secrets() {
        assert!(is_sensitive_path(Path::new("/etc/shadow")));
        assert!(is_sensitive_path(Path::new("/etc/gshadow")));
    }

    #[test]
    fn test_is_sensitive_path_blocks_key_files_by_extension() {
        assert!(is_sensitive_path(Path::new("/tmp/server.pem")));
        assert!(is_sensitive_path(Path::new("/app/certs/private.key")));
        assert!(is_sensitive_path(Path::new("/home/user/keystore.p12")));
    }

    #[test]
    fn test_is_sensitive_path_allows_safe_suffixes() {
        assert!(!is_sensitive_path(Path::new("/app/.env.example")));
        assert!(!is_sensitive_path(Path::new("/app/.env.sample")));
        assert!(!is_sensitive_path(Path::new("/app/.env.template")));
    }

    #[test]
    fn test_is_sensitive_path_allows_normal_files() {
        assert!(!is_sensitive_path(Path::new("/app/src/main.rs")));
        assert!(!is_sensitive_path(Path::new("/home/user/README.md")));
        assert!(!is_sensitive_path(Path::new("/tmp/output.json")));
    }

    #[test]
    fn test_is_sensitive_path_blocks_env_files() {
        assert!(is_sensitive_path(Path::new("/app/.env")));
        assert!(is_sensitive_path(Path::new("/app/.env.local")));
        assert!(is_sensitive_path(Path::new("/app/.env.production")));
    }

    // ── command scanning tests ──

    #[test]
    fn test_command_references_sensitive_path_catches_cat_ssh() {
        assert!(command_references_sensitive_path("cat ~/.ssh/id_rsa").is_some());
        assert!(
            command_references_sensitive_path("head -n 5 /home/user/.ssh/authorized_keys")
                .is_some()
        );
    }

    #[test]
    fn test_command_references_sensitive_path_catches_aws() {
        assert!(command_references_sensitive_path("cat ~/.aws/credentials").is_some());
        assert!(command_references_sensitive_path("grep key ~/.aws/config").is_some());
    }

    #[test]
    fn test_command_references_sensitive_path_catches_etc_shadow() {
        assert!(command_references_sensitive_path("cat /etc/shadow").is_some());
    }

    #[test]
    fn test_command_references_sensitive_path_catches_key_extensions() {
        assert!(command_references_sensitive_path("cp server.pem /tmp/").is_some());
        assert!(command_references_sensitive_path("cat private.key").is_some());
    }

    #[test]
    fn test_command_references_sensitive_path_allows_safe_commands() {
        assert!(command_references_sensitive_path("ls -la").is_none());
        assert!(command_references_sensitive_path("cargo build").is_none());
        assert!(command_references_sensitive_path("git status").is_none());
        assert!(command_references_sensitive_path("cat README.md").is_none());
    }

    // ── Issue 1: .env must not match .envrc or .environment ──

    #[test]
    fn test_envrc_not_blocked_by_path_check() {
        assert!(!is_sensitive_path(Path::new("/app/.envrc")));
        assert!(!is_sensitive_path(Path::new("/home/user/.environment")));
        assert!(!is_sensitive_path(Path::new("/project/.envrc.local")));
    }

    #[test]
    fn test_envrc_not_blocked_by_command_scanner() {
        assert!(command_references_sensitive_path("cat /app/.envrc").is_none());
        assert!(command_references_sensitive_path("source .envrc").is_none());
        assert!(command_references_sensitive_path("direnv allow /project/.envrc").is_none());
        assert!(command_references_sensitive_path("cat /app/.environment").is_none());
    }

    #[test]
    fn test_env_files_still_blocked_after_envrc_fix() {
        // Exact .env
        assert!(is_sensitive_path(Path::new("/app/.env")));
        assert!(command_references_sensitive_path("cat /app/.env").is_some());
        // .env.local, .env.production etc.
        assert!(is_sensitive_path(Path::new("/app/.env.local")));
        assert!(command_references_sensitive_path("cat /app/.env.production").is_some());
    }

    // ── Issue 2: extension matching must not be substring-based in commands ──

    #[test]
    fn test_key_flag_not_blocked() {
        // --key-file is a flag, not a .key file
        assert!(command_references_sensitive_path("--key-file /tmp/foo").is_none());
        // --keystore-password is a flag, not a .keystore file
        assert!(
            command_references_sensitive_path("java --keystore-password=abc main.jar").is_none()
        );
        // But actual .key files should still be blocked
        assert!(command_references_sensitive_path("cat private.key").is_some());
        assert!(command_references_sensitive_path("cp server.pem /backup/").is_some());
    }

    #[test]
    fn test_extension_at_end_of_token_still_blocked() {
        assert!(command_references_sensitive_path("scp host:cert.pem .").is_some());
        assert!(command_references_sensitive_path("cat /certs/tls.key").is_some());
        assert!(command_references_sensitive_path("keytool -import -file store.p12").is_some());
    }
}
