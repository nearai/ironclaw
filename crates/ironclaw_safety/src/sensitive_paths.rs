//! Filesystem path sensitivity checking.
//!
//! Blocks access to credential-bearing files and directories to prevent
//! information leakage through file tools.

use std::path::Path;

/// Path patterns that indicate sensitive credential stores.
const SENSITIVE_PATH_PATTERNS: &[&str] = &[
    "/.ssh/",
    "/.aws/",
    "/.netrc",
    "/.pgpass",
    "/.npmrc",
    "/.pypirc",
    "/.docker/",
    "/.kube/",
    "/.git-credentials",
    "/.gcloud/",
    "/.config/gcloud/",
    "/.gnupg/",
    "/.vault-token",
    "/.ironclaw/secrets/",
    "/.config/gh/hosts.yml",
    "/etc/shadow",
    "/etc/gshadow",
    "/.terraform.d/credentials.tfrc.json",
    "/.azure/",
    // Shell history files
    "/.bash_history",
    "/.zsh_history",
    "/.histfile",
];

/// Sensitive file extensions that indicate cryptographic key material.
const SENSITIVE_EXTENSIONS: &[&str] = &[".pem", ".key", ".p12", ".pfx", ".jks", ".keystore"];

/// Safe file suffixes that should NOT be blocked even if they match sensitive extensions.
const SAFE_SUFFIXES: &[&str] = &[".dist"];

/// Safe `.env` file suffixes that should NOT be blocked.
const ENV_SAFE_SUFFIXES: &[&str] = &[".example", ".template", ".sample", ".dist"];

/// Sensitive filenames that indicate SSH key material or access control files.
/// Checked via exact filename match (not substring) to avoid false positives
/// on paths like `/project/grid_rsa_data`.
const SENSITIVE_FILENAMES: &[&str] = &[
    "id_rsa",
    "id_ed25519",
    "id_ecdsa",
    "id_dsa",
    "authorized_keys",
    "known_hosts",
];

/// Check whether a filesystem path points to a sensitive credential file or directory.
///
/// Operates on the string representation of the path after normalizing separators
/// and lowercasing. Callers should pass canonicalized paths (after symlink resolution)
/// to prevent symlink-based bypass.
///
/// Note: `canonicalize()` is a blocking syscall. On local filesystems this is
/// sub-millisecond and acceptable in async context. On network filesystems
/// (NFS, CIFS) it could block the tokio runtime. If this becomes a problem,
/// make this function async and use `tokio::fs::canonicalize`.
pub fn is_sensitive_path(path: &Path) -> bool {
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let path_str = resolved
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();

    // Block .env files (except safe suffixes like .env.example)
    // Must NOT match .envrc, .environment, etc. — only exact ".env" or ".env.<variant>"
    if let Some(filename) = resolved.file_name().and_then(|f| f.to_str()) {
        let filename_lower = filename.to_ascii_lowercase();
        if filename_lower == ".env" || filename_lower.starts_with(".env.") {
            let remainder = filename_lower.strip_prefix(".env").unwrap_or("");
            let is_safe = ENV_SAFE_SUFFIXES.contains(&remainder);
            if !is_safe {
                return true;
            }
        }
    }

    // Check sensitive file extensions (e.g. .pem, .key, .p12)
    if let Some(filename) = resolved.file_name().and_then(|f| f.to_str()) {
        let filename_lower = filename.to_ascii_lowercase();
        // Skip files with safe suffixes (e.g. server.key.dist)
        let has_safe_suffix = SAFE_SUFFIXES.iter().any(|s| filename_lower.ends_with(s));
        if !has_safe_suffix {
            let has_sensitive_ext = SENSITIVE_EXTENSIONS
                .iter()
                .any(|ext| filename_lower.ends_with(ext));
            if has_sensitive_ext {
                return true;
            }
        }
    }

    // Check sensitive filenames by exact match (e.g. id_rsa, authorized_keys)
    if let Some(filename) = resolved.file_name().and_then(|f| f.to_str()) {
        let filename_lower = filename.to_ascii_lowercase();
        if SENSITIVE_FILENAMES.iter().any(|&f| filename_lower == f) {
            return true;
        }
    }

    // Check sensitive path patterns.
    // Also check with a trailing slash so that directory paths (e.g. "/home/.ssh")
    // match patterns that require a trailing slash (e.g. "/.ssh/").
    let path_str_with_slash = format!("{}/", path_str);
    SENSITIVE_PATH_PATTERNS.iter().any(|p| {
        let lower = p.to_ascii_lowercase();
        path_str.contains(&lower) || path_str_with_slash.contains(&lower)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn blocks_dotenv() {
        assert!(is_sensitive_path(Path::new("/home/user/.env")));
        assert!(is_sensitive_path(Path::new("/home/user/.env.local")));
        assert!(is_sensitive_path(Path::new("/home/user/.env.production")));
    }

    #[test]
    fn allows_env_safe_suffixes() {
        assert!(!is_sensitive_path(Path::new("/home/user/.env.example")));
        assert!(!is_sensitive_path(Path::new("/home/user/.env.template")));
        assert!(!is_sensitive_path(Path::new("/home/user/.env.sample")));
    }

    #[test]
    fn blocks_env_with_compound_suffix() {
        // .env.production.dist must NOT be allowed: the remainder ".production.dist"
        // is not an exact match for any safe suffix like ".dist".
        assert!(is_sensitive_path(Path::new("/app/.env.production.dist")));
    }

    #[test]
    fn blocks_ssh() {
        assert!(is_sensitive_path(Path::new("/home/user/.ssh/id_rsa")));
        assert!(is_sensitive_path(Path::new("/home/user/.ssh/config")));
        assert!(is_sensitive_path(Path::new(
            "/home/user/.ssh/authorized_keys"
        )));
    }

    #[test]
    fn blocks_aws_directory() {
        assert!(is_sensitive_path(Path::new("/home/user/.aws/credentials")));
        assert!(is_sensitive_path(Path::new("/home/user/.aws/config")));
        // Directory-level blocking: any file under .aws/ is sensitive
        assert!(is_sensitive_path(Path::new(
            "/home/user/.aws/cli/cache/token.json"
        )));
    }

    #[test]
    fn blocks_docker_directory() {
        assert!(is_sensitive_path(Path::new(
            "/home/user/.docker/config.json"
        )));
        assert!(is_sensitive_path(Path::new(
            "/home/user/.docker/daemon.json"
        )));
    }

    #[test]
    fn blocks_kube_directory() {
        assert!(is_sensitive_path(Path::new("/home/user/.kube/config")));
        assert!(is_sensitive_path(Path::new(
            "/home/user/.kube/cache/tokens"
        )));
    }

    #[test]
    fn blocks_new_paths() {
        assert!(is_sensitive_path(Path::new(
            "/home/user/.config/gh/hosts.yml"
        )));
        assert!(is_sensitive_path(Path::new("/etc/shadow")));
        assert!(is_sensitive_path(Path::new("/etc/gshadow")));
        assert!(is_sensitive_path(Path::new(
            "/home/user/.terraform.d/credentials.tfrc.json"
        )));
        assert!(is_sensitive_path(Path::new("/home/user/.azure/config")));
    }

    #[test]
    fn blocks_shell_history() {
        assert!(is_sensitive_path(Path::new("/home/user/.bash_history")));
        assert!(is_sensitive_path(Path::new("/home/user/.zsh_history")));
        assert!(is_sensitive_path(Path::new("/home/user/.histfile")));
    }

    #[test]
    fn blocks_ssh_key_types() {
        assert!(is_sensitive_path(Path::new("/home/user/.ssh/id_rsa")));
        assert!(is_sensitive_path(Path::new("/home/user/.ssh/id_ed25519")));
        assert!(is_sensitive_path(Path::new("/home/user/.ssh/id_ecdsa")));
        assert!(is_sensitive_path(Path::new("/home/user/.ssh/id_dsa")));
        assert!(is_sensitive_path(Path::new(
            "/home/user/.ssh/authorized_keys"
        )));
        assert!(is_sensitive_path(Path::new("/home/user/.ssh/known_hosts")));
    }

    #[test]
    fn blocks_standalone_id_rsa() {
        // id_rsa outside of .ssh/ directory should still be caught
        assert!(is_sensitive_path(Path::new("/tmp/backup/id_rsa")));
    }

    #[test]
    fn blocks_sensitive_filename_in_test_fixtures() {
        // Even inside test_fixtures, a file literally named id_rsa is sensitive
        assert!(is_sensitive_path(Path::new(
            "/project/test_fixtures/id_rsa"
        )));
    }

    #[test]
    fn allows_substring_of_sensitive_filename() {
        // grid_rsa_data contains "id_rsa" as a substring but is NOT the filename
        assert!(!is_sensitive_path(Path::new("/project/grid_rsa_data")));
    }

    #[test]
    fn blocks_sensitive_extensions() {
        assert!(is_sensitive_path(Path::new("/home/user/server.pem")));
        assert!(is_sensitive_path(Path::new("/home/user/server.key")));
        assert!(is_sensitive_path(Path::new("/home/user/cert.p12")));
        assert!(is_sensitive_path(Path::new("/home/user/keystore.pfx")));
        assert!(is_sensitive_path(Path::new("/home/user/app.jks")));
        assert!(is_sensitive_path(Path::new("/home/user/my.keystore")));
    }

    #[test]
    fn allows_safe_dist_suffix() {
        assert!(!is_sensitive_path(Path::new("/home/user/server.key.dist")));
        assert!(!is_sensitive_path(Path::new("/home/user/cert.pem.dist")));
    }

    #[test]
    fn blocks_other_credential_stores() {
        assert!(is_sensitive_path(Path::new("/home/user/.netrc")));
        assert!(is_sensitive_path(Path::new("/home/user/.npmrc")));
        assert!(is_sensitive_path(Path::new("/home/user/.pgpass")));
        assert!(is_sensitive_path(Path::new("/home/user/.kube/config")));
        assert!(is_sensitive_path(Path::new("/home/user/.git-credentials")));
        assert!(is_sensitive_path(Path::new(
            "/home/user/.docker/config.json"
        )));
        assert!(is_sensitive_path(Path::new(
            "/home/user/.gnupg/private-keys-v1.d/key.gpg"
        )));
        assert!(is_sensitive_path(Path::new("/home/user/.vault-token")));
        assert!(is_sensitive_path(Path::new(
            "/home/user/.ironclaw/secrets/keys.json"
        )));
    }

    #[test]
    fn allows_normal_files() {
        assert!(!is_sensitive_path(Path::new("/home/user/code/main.rs")));
        assert!(!is_sensitive_path(Path::new("/home/user/docs/readme.md")));
        assert!(!is_sensitive_path(Path::new("/tmp/test.txt")));
    }

    #[test]
    fn env_does_not_match_envrc_or_environment() {
        assert!(!is_sensitive_path(Path::new("/home/user/.envrc")));
        assert!(!is_sensitive_path(Path::new("/home/user/.environment")));
        assert!(!is_sensitive_path(Path::new("/home/user/project/.envrc")));
    }

    #[test]
    fn case_insensitive() {
        assert!(is_sensitive_path(Path::new("/home/user/.SSH/id_rsa")));
        assert!(is_sensitive_path(Path::new("/home/user/.ENV")));
    }

    #[test]
    fn path_traversal_caught() {
        // These won't canonicalize to real paths in test, but the string matching
        // should still catch the patterns after normalization
        let traversal = PathBuf::from("/home/user/project/../../user/.ssh/id_rsa");
        // If canonicalize fails (path doesn't exist), falls back to raw path
        // The raw path still contains /.ssh/ so it should be caught
        assert!(is_sensitive_path(&traversal));
    }
}
