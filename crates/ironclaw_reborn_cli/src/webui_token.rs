//! Shared WebChat v2 bearer token resolution and provisioning.
//!
//! The token doubles as both the WebChat v2 env-bearer credential *and*
//! the stateless session-signing HMAC key (see `commands/serve.rs`), so
//! it must be high-entropy (>= [`WEBUI_TOKEN_MIN_BYTES`]) regardless of
//! whether it comes from the environment or the onboarding-provisioned
//! `<reborn_home>/webui-token` fallback file.
//!
//! `onboard` (unconditional — [`ensure_webui_token_file`]) provisions the
//! fallback file so a service-installed `serve` (launchd/systemd), whose
//! unit environment carries only `HOME`/`PROFILE` (see
//! `serve_invocation.rs`), still has a token to read. `serve` and `tui`
//! (both gated behind the `webui-v2-beta` feature — [`resolve_webui_token`])
//! read through the same precedence and entropy validation so the two
//! commands can never silently disagree on how a token resolves.

use std::fs;
use std::path::{Path, PathBuf};

use crate::file_write::FileWriteAction;

/// Filename of the onboarding-provisioned WebChat v2 bearer token,
/// relative to `<reborn_home>`. Shared by `onboard` (which writes it)
/// and `serve`/`tui` (which read it as a fallback when the env var
/// naming the token is unset or empty).
pub(crate) const WEBUI_TOKEN_FILENAME: &str = "webui-token";

/// Minimum byte length for the WebChat v2 bearer token, mirroring the
/// server-side session-signing entropy floor: an attacker who obtains
/// one legitimate signed session can brute-force a low-entropy key
/// offline, then mint a session for any user/tenant.
pub(crate) const WEBUI_TOKEN_MIN_BYTES: usize = 32;

/// Absolute path of the onboarding-provisioned token file under
/// `<reborn_home>`.
pub(crate) fn webui_token_file_path(reborn_home: &Path) -> PathBuf {
    reborn_home.join(WEBUI_TOKEN_FILENAME)
}

/// `true` when a token file exists at `<reborn_home>/webui-token` and
/// its trimmed contents meet the entropy floor. Read-only — used both
/// by [`ensure_webui_token_file`] (to decide whether to skip writing)
/// and by `onboard --dry-run` (to report what it *would* do).
pub(crate) fn webui_token_file_is_valid(reborn_home: &Path) -> bool {
    fs::read_to_string(webui_token_file_path(reborn_home))
        .map(|contents| contents.trim().len() >= WEBUI_TOKEN_MIN_BYTES)
        .unwrap_or(false)
}

/// Ensure `<reborn_home>/webui-token` holds a valid (>= entropy floor)
/// token, generating and writing one with `0600` permissions (unix) if
/// none exists yet.
///
/// Idempotent by design, independent of any `--force` flag: a valid
/// existing token is never regenerated, because operators may already
/// have long-lived sessions or an externally-copied env var keyed to
/// its current value. Only a missing or invalid (too-short/unreadable)
/// file is (re)written.
pub(crate) fn ensure_webui_token_file(reborn_home: &Path) -> anyhow::Result<FileWriteAction> {
    let file_path = webui_token_file_path(reborn_home);
    if webui_token_file_is_valid(reborn_home) {
        return Ok(FileWriteAction::Preserved);
    }
    let overwrote = file_path.exists();
    let token = generate_webui_token();
    write_token_file(&file_path, &token)?;
    Ok(if overwrote {
        FileWriteAction::Overwrote
    } else {
        FileWriteAction::Wrote
    })
}

/// Generate a cryptographically-random token comfortably over the
/// entropy floor: [`WEBUI_TOKEN_MIN_BYTES`] random bytes, hex-encoded
/// (twice the byte length as ASCII characters).
fn generate_webui_token() -> String {
    use rand::RngExt as _;
    let mut bytes = [0_u8; WEBUI_TOKEN_MIN_BYTES];
    rand::rng().fill(&mut bytes);
    hex::encode(bytes)
}

/// Write `token` to `path` atomically (temp file + rename) with `0600`
/// permissions on unix, creating the parent directory if needed.
fn write_token_file(path: &Path, token: &str) -> anyhow::Result<()> {
    use std::io::Write as _;

    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| anyhow::anyhow!("create {}: {error}", parent.display()))?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| anyhow::anyhow!("create temp file in {}: {error}", parent.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        tmp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|error| {
                anyhow::anyhow!("set permissions on {}: {error}", tmp.path().display())
            })?;
    }
    tmp.write_all(token.as_bytes())
        .map_err(|error| anyhow::anyhow!("write {}: {error}", tmp.path().display()))?;
    tmp.flush()
        .map_err(|error| anyhow::anyhow!("flush {}: {error}", tmp.path().display()))?;
    tmp.persist(path).map_err(|error| {
        anyhow::anyhow!(
            "persist {} -> {}: {}",
            error.file.path().display(),
            path.display(),
            error.error
        )
    })?;
    Ok(())
}

/// Resolve the WebChat v2 bearer token with precedence:
///
/// 1. `env_value` — the value of the operator's `[webui].env_token_var`
///    (default `IRONCLAW_REBORN_WEBUI_TOKEN`) — when `Some` and
///    non-empty;
/// 2. `<reborn_home>/webui-token`, trimmed, when it exists and is
///    non-empty (the `onboard`-provisioned fallback — see the module
///    doc for why a service-installed `serve` needs this);
/// 3. otherwise a fail-closed error naming both the env var and the
///    fallback file path.
///
/// Either source must independently meet [`WEBUI_TOKEN_MIN_BYTES`] — a
/// short value fails closed rather than letting `serve` start and the
/// server reject it opaquely later.
///
/// Pure: takes the env value and home path as parameters instead of
/// reading `std::env` itself, so callers own env access (trivially
/// unit-testable without mutating process env) and control which env
/// var name is in play (`[webui].env_token_var` may rename it).
#[cfg(feature = "webui-v2-beta")]
pub(crate) fn resolve_webui_token(
    env_var_name: &str,
    env_value: Option<&str>,
    reborn_home: &Path,
) -> anyhow::Result<String> {
    if let Some(value) = env_value
        && !value.is_empty()
    {
        validate_token_entropy(value, env_var_name, reborn_home)?;
        return Ok(value.to_string());
    }

    let file_path = webui_token_file_path(reborn_home);
    let file_value = fs::read_to_string(&file_path)
        .ok()
        .map(|contents| contents.trim().to_string())
        .filter(|trimmed| !trimmed.is_empty());

    match file_value {
        Some(token) => {
            validate_token_entropy(&token, env_var_name, reborn_home)?;
            Ok(token)
        }
        None => Err(anyhow::anyhow!(
            "{env_var_name} must be set to the WebChat v2 bearer token, or a token file must \
             exist at {} (written by `ironclaw-reborn onboard`). Neither was found.",
            file_path.display()
        )),
    }
}

#[cfg(feature = "webui-v2-beta")]
fn validate_token_entropy(
    value: &str,
    env_var_name: &str,
    reborn_home: &Path,
) -> anyhow::Result<()> {
    if value.len() >= WEBUI_TOKEN_MIN_BYTES {
        return Ok(());
    }
    Err(anyhow::anyhow!(
        "the WebChat v2 bearer token (from {env_var_name} or {}) is only {} bytes; it is also \
         the WebChat v2 session-signing key and must be at least {WEBUI_TOKEN_MIN_BYTES} bytes \
         of high-entropy random material. Generate one with e.g. `openssl rand -hex 32`, or run \
         `ironclaw-reborn onboard` to provision a valid token file.",
        webui_token_file_path(reborn_home).display(),
        value.len(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_TOKEN: &str = "reborn-smoke-test-token-0123456789abcdef"; // 40 bytes

    #[test]
    fn ensure_webui_token_file_creates_valid_token_with_0600_perms() {
        let dir = tempfile::tempdir().expect("tempdir");
        let action = ensure_webui_token_file(dir.path()).expect("token file should be created");
        assert_eq!(action, FileWriteAction::Wrote);

        let path = webui_token_file_path(dir.path());
        let contents = fs::read_to_string(&path).expect("read token file");
        assert!(
            contents.trim().len() >= WEBUI_TOKEN_MIN_BYTES,
            "generated token must meet the entropy floor: {} bytes",
            contents.trim().len()
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(&path)
                .expect("stat token file")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "token file must be 0600, got {mode:o}");
        }
    }

    #[test]
    fn ensure_webui_token_file_is_idempotent_when_existing_token_is_valid() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = ensure_webui_token_file(dir.path()).expect("first write");
        assert_eq!(first, FileWriteAction::Wrote);
        let path = webui_token_file_path(dir.path());
        let first_contents = fs::read_to_string(&path).expect("read token file");

        let second = ensure_webui_token_file(dir.path()).expect("second call must not fail");
        assert_eq!(
            second,
            FileWriteAction::Preserved,
            "a valid existing token must never be clobbered"
        );
        let second_contents = fs::read_to_string(&path).expect("read token file again");
        assert_eq!(
            first_contents, second_contents,
            "idempotent onboard must not regenerate a valid token"
        );
    }

    #[test]
    fn ensure_webui_token_file_replaces_a_too_short_existing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = webui_token_file_path(dir.path());
        fs::write(&path, "too-short\n").expect("seed short token file");

        let action = ensure_webui_token_file(dir.path()).expect("should regenerate");
        assert_eq!(action, FileWriteAction::Overwrote);
        let contents = fs::read_to_string(&path).expect("read token file");
        assert!(contents.trim().len() >= WEBUI_TOKEN_MIN_BYTES);
    }

    #[cfg(feature = "webui-v2-beta")]
    #[test]
    fn resolve_prefers_env_value_when_set() {
        let dir = tempfile::tempdir().expect("tempdir");
        let token = resolve_webui_token("SOME_TOKEN_VAR", Some(VALID_TOKEN), dir.path())
            .expect("env value should resolve");
        assert_eq!(token, VALID_TOKEN);
    }

    #[cfg(feature = "webui-v2-beta")]
    #[test]
    fn resolve_falls_back_to_home_file_when_env_unset() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = webui_token_file_path(dir.path());
        fs::write(&path, format!("  {VALID_TOKEN}  \n")).expect("seed token file");

        let token = resolve_webui_token("SOME_TOKEN_VAR", None, dir.path())
            .expect("file fallback should resolve");
        assert_eq!(token, VALID_TOKEN, "file value must be trimmed");
    }

    #[cfg(feature = "webui-v2-beta")]
    #[test]
    fn resolve_errors_naming_both_var_and_file_when_neither_is_present() {
        let dir = tempfile::tempdir().expect("tempdir");
        let error = resolve_webui_token("SOME_TOKEN_VAR", None, dir.path())
            .expect_err("neither source is present");
        let message = error.to_string();
        assert!(
            message.contains("SOME_TOKEN_VAR must be set"),
            "message: {message}"
        );
        assert!(
            message.contains(&webui_token_file_path(dir.path()).display().to_string()),
            "message: {message}"
        );
    }

    #[cfg(feature = "webui-v2-beta")]
    #[test]
    fn resolve_rejects_a_too_short_env_token() {
        let dir = tempfile::tempdir().expect("tempdir");
        let error = resolve_webui_token("SOME_TOKEN_VAR", Some("short"), dir.path())
            .expect_err("short env token must be rejected");
        let message = error.to_string();
        assert!(
            message.contains("session-signing key"),
            "message: {message}"
        );
        assert!(message.contains("at least 32 bytes"), "message: {message}");
    }

    #[cfg(feature = "webui-v2-beta")]
    #[test]
    fn resolve_rejects_a_too_short_home_file_token() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = webui_token_file_path(dir.path());
        fs::write(&path, "short\n").expect("seed short token file");

        let error = resolve_webui_token("SOME_TOKEN_VAR", None, dir.path())
            .expect_err("short file token must be rejected");
        let message = error.to_string();
        assert!(
            message.contains("session-signing key"),
            "message: {message}"
        );
        assert!(message.contains("at least 32 bytes"), "message: {message}");
    }
}
