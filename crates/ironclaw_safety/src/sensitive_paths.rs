//! Filesystem path sensitivity checking.
//!
//! Blocks access to credential-bearing files and directories to prevent
//! information leakage through file tools.

use std::path::Path;

/// Path patterns that indicate sensitive credential stores.
const SENSITIVE_PATH_PATTERNS: &[&str] = &[
    "/.ssh/",
    "/.aws/credentials",
    "/.aws/config",
    "/.netrc",
    "/.pgpass",
    "/.npmrc",
    "/.pypirc",
    "/.docker/config.json",
    "/.kube/config",
    "/.git-credentials",
    "/.gcloud/",
    "/.config/gcloud/",
    "/.gnupg/",
    "/.vault-token",
    "/.ironclaw/secrets/",
    // Additional paths from reviewer feedback
    "/.config/gh/hosts.yml",
    "/etc/shadow",
    "/.terraform.d/credentials.tfrc.json",
    "/.azure/",
];

/// Safe `.env` file suffixes that should NOT be blocked.
const ENV_SAFE_SUFFIXES: &[&str] = &[".example", ".template", ".sample"];

/// Check whether a filesystem path points to a sensitive credential file or directory.
///
/// Operates on the string representation of the path after normalizing separators
/// and lowercasing. Callers should pass canonicalized paths (after symlink resolution)
/// to prevent symlink-based bypass.
pub fn is_sensitive_path(path: &Path) -> bool {
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let path_str = resolved
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();

    // Block .env files (except safe suffixes like .env.example)
    if let Some(filename) = resolved.file_name().and_then(|f| f.to_str()) {
        let filename_lower = filename.to_ascii_lowercase();
        if filename_lower == ".env" || filename_lower.starts_with(".env.") {
            let is_safe = ENV_SAFE_SUFFIXES
                .iter()
                .any(|suffix| filename_lower.ends_with(suffix));
            if !is_safe {
                return true;
            }
        }
    }

    // Check sensitive path patterns
    SENSITIVE_PATH_PATTERNS
        .iter()
        .any(|p| path_str.contains(&p.to_ascii_lowercase()))
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
    fn blocks_ssh() {
        assert!(is_sensitive_path(Path::new("/home/user/.ssh/id_rsa")));
        assert!(is_sensitive_path(Path::new("/home/user/.ssh/config")));
        assert!(is_sensitive_path(Path::new(
            "/home/user/.ssh/authorized_keys"
        )));
    }

    #[test]
    fn blocks_aws() {
        assert!(is_sensitive_path(Path::new("/home/user/.aws/credentials")));
        assert!(is_sensitive_path(Path::new("/home/user/.aws/config")));
    }

    #[test]
    fn blocks_new_paths() {
        assert!(is_sensitive_path(Path::new(
            "/home/user/.config/gh/hosts.yml"
        )));
        assert!(is_sensitive_path(Path::new("/etc/shadow")));
        assert!(is_sensitive_path(Path::new(
            "/home/user/.terraform.d/credentials.tfrc.json"
        )));
        assert!(is_sensitive_path(Path::new("/home/user/.azure/config")));
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
