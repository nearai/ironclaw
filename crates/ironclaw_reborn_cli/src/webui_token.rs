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
//! `serve_invocation.rs`), still has a token to read. `serve` (gated
//! behind the `webui-v2-beta` feature — [`resolve_webui_token`]) reads
//! through this same precedence and entropy validation.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context as _;

use crate::file_write::FileWriteAction;

/// Filename of the onboarding-provisioned WebChat v2 bearer token,
/// relative to `<reborn_home>`. Shared by `onboard` (which writes it)
/// and `serve` (which reads it as a fallback when the env var naming
/// the token is unset or empty).
pub(crate) const WEBUI_TOKEN_FILENAME: &str = "webui-token";

/// Default name of the env var that overrides the token file at runtime,
/// absent an operator override via `[webui].env_token_var`. Shared by
/// `serve` (which reads it) and `config set webui.token --rotate` (which
/// checks it before rotating, since rotating the file has no effect while
/// this env var is set and non-empty — see `commands::config::set`).
pub(crate) const DEFAULT_ENV_TOKEN_VAR: &str = "IRONCLAW_REBORN_WEBUI_TOKEN";

/// Minimum byte length for the WebChat v2 bearer token, mirroring the
/// server-side session-signing entropy floor: an attacker who obtains
/// one legitimate signed session can brute-force a low-entropy key
/// offline, then mint a session for any user/tenant.
pub(crate) const WEBUI_TOKEN_MIN_BYTES: usize = 32;

/// Upper bound on the on-disk size of the token file before we refuse to
/// read it. A legitimate token (hex-encoded, [`WEBUI_TOKEN_MIN_BYTES`]
/// bytes) plus generous whitespace is a few dozen bytes; this cap is wide
/// headroom over that while still bounding memory use against an
/// oversized or corrupt file, since the read is otherwise unbounded.
const WEBUI_TOKEN_FILE_MAX_BYTES: u64 = 4096;

/// Absolute path of the onboarding-provisioned token file under
/// `<reborn_home>`.
pub(crate) fn webui_token_file_path(reborn_home: &Path) -> PathBuf {
    reborn_home.join(WEBUI_TOKEN_FILENAME)
}

/// Read the token file's raw contents through the shared safety checks:
/// reject a symlink (it could point outside `<reborn_home>` at a file the
/// operator does not control) and reject a file over
/// [`WEBUI_TOKEN_FILE_MAX_BYTES`] (bound the read) before trusting the
/// path enough to read it. `Ok(None)` means "no file here" (`NotFound`
/// only); every other I/O failure — permission denied, a directory at
/// this path, etc. — is a real error and must propagate rather than
/// silently reading as absent, which would let a caller like
/// [`ensure_webui_token_file`] overwrite an existing-but-unreadable
/// secret.
///
/// On unix this opens the path exactly once with `O_NOFOLLOW` and drives
/// every subsequent check (regular-file type, size, contents) off that
/// same handle's `fstat`/read — a symlink swapped in, or the target
/// enlarged, after a separate `symlink_metadata()` call but before the
/// read can't smuggle a different file through (TOCTOU), and a non-regular
/// file (FIFO, device) that would otherwise pass a standalone length check
/// and then block the read indefinitely is rejected before any read is
/// attempted. Non-unix targets keep the previous best-effort
/// stat-then-read shape (no `O_NOFOLLOW`/single-handle primitive in
/// `std` there).
#[cfg(unix)]
fn read_token_file_checked(path: &Path) -> anyhow::Result<Option<String>> {
    use std::io::Read as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    // `O_NONBLOCK` alongside `O_NOFOLLOW`: a read-only open of a FIFO
    // blocks until a writer connects, which would otherwise hang `serve`
    // startup indefinitely on a FIFO planted at the token path (it would
    // have passed a standalone pre-read length check too). `O_NONBLOCK`
    // makes the open return immediately regardless of a writer; it has
    // no effect on the regular-file case once we reach the read below
    // (POSIX ignores O_NONBLOCK for regular files).
    let mut options = fs::OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        // `O_NOFOLLOW` surfaces a symlink at `path` as `ELOOP`
        // (`ErrorKind::FilesystemLoop` on recent stds, `Uncategorized`/
        // `Other` on older ones) rather than a distinct error kind we can
        // match reliably across platforms/std versions — so probe
        // `symlink_metadata` only on open failure to produce the
        // targeted message, instead of trusting `open`'s raw errno
        // classification.
        Err(open_error) => {
            if fs::symlink_metadata(path)
                .map(|metadata| metadata.file_type().is_symlink())
                .unwrap_or(false)
            {
                anyhow::bail!(
                    "{} is a symlink; refusing to read the WebChat v2 token file through it. \
                     Remove it and re-run `ironclaw onboard` to provision a regular file.",
                    path.display()
                );
            }
            return Err(open_error).with_context(|| format!("open {}", path.display()));
        }
    };

    let metadata = file
        .metadata()
        .with_context(|| format!("stat {}", path.display()))?;
    if !metadata.is_file() {
        anyhow::bail!(
            "{} is not a regular file; refusing to read the WebChat v2 token file through it. \
             Remove it and re-run `ironclaw onboard` to provision a regular file.",
            path.display()
        );
    }
    if metadata.len() > WEBUI_TOKEN_FILE_MAX_BYTES {
        anyhow::bail!(
            "{} is {} bytes, over the {WEBUI_TOKEN_FILE_MAX_BYTES}-byte cap for the \
             WebChat v2 token file; refusing to read it.",
            path.display(),
            metadata.len()
        );
    }

    // Bound the read itself (not just the pre-read `fstat`'d size) to
    // `MAX_BYTES + 1`: a file that grows between the `fstat` above and
    // this read (e.g. concurrently appended to) is still caught here
    // instead of being read unbounded.
    let mut buf = Vec::new();
    (&file)
        .take(WEBUI_TOKEN_FILE_MAX_BYTES + 1)
        .read_to_end(&mut buf)
        .with_context(|| format!("read {}", path.display()))?;
    if buf.len() as u64 > WEBUI_TOKEN_FILE_MAX_BYTES {
        anyhow::bail!(
            "{} exceeds the {WEBUI_TOKEN_FILE_MAX_BYTES}-byte cap for the WebChat v2 token \
             file; refusing to read it.",
            path.display()
        );
    }
    let contents = String::from_utf8(buf)
        .map_err(|_| anyhow::anyhow!("{} is not valid UTF-8", path.display()))?;
    Ok(Some(contents))
}

#[cfg(not(unix))]
fn read_token_file_checked(path: &Path) -> anyhow::Result<Option<String>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                anyhow::bail!(
                    "{} is a symlink; refusing to read the WebChat v2 token file through it. \
                     Remove it and re-run `ironclaw onboard` to provision a regular file.",
                    path.display()
                );
            }
            if metadata.len() > WEBUI_TOKEN_FILE_MAX_BYTES {
                anyhow::bail!(
                    "{} is {} bytes, over the {WEBUI_TOKEN_FILE_MAX_BYTES}-byte cap for the \
                     WebChat v2 token file; refusing to read it.",
                    path.display(),
                    metadata.len()
                );
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("stat {}", path.display())),
    }
    match fs::read_to_string(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

/// Repair a token file's permissions to `0600` if they aren't already,
/// logging a warning. Called only from the "accept this token" paths
/// ([`ensure_webui_token_file`]'s preserve branch, [`resolve_webui_token`]'s
/// file-fallback branch) — never from a read-only reporting path like
/// `onboard --dry-run` — on the principle that a wrongly-permissioned but
/// otherwise valid token should be repaired in place rather than
/// rejected (repair-over-reject: least user breakage, and it matches the
/// 0600 discipline [`write_token_file`] already enforces for freshly
/// written tokens).
#[cfg(unix)]
fn repair_token_file_mode(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let metadata =
        fs::metadata(path).with_context(|| format!("stat {} for mode repair", path.display()))?;
    let mode = metadata.permissions().mode() & 0o777;
    if mode == 0o600 {
        return Ok(());
    }
    tracing::warn!(
        target: "ironclaw::reborn::cli::webui_token",
        path = %path.display(),
        mode = format!("{mode:o}"),
        "repairing WebChat v2 token file permissions to 0600"
    );
    fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("repair permissions on {}", path.display()))
}

#[cfg(not(unix))]
fn repair_token_file_mode(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

/// `Ok(true)` when a token file exists at `<reborn_home>/webui-token` and
/// its trimmed contents meet the entropy floor; `Ok(false)` when it is
/// absent or too short. Read-only (no mode repair, no writes) — used by
/// `onboard --dry-run` to report what it *would* do without mutating
/// anything. [`ensure_webui_token_file`] additionally repairs the mode
/// on its own accept path rather than relying on this helper for that.
///
/// Propagates a real error for any I/O failure that isn't "file absent"
/// (unreadable file, symlink, oversized file) — see
/// [`read_token_file_checked`].
pub(crate) fn webui_token_file_is_valid(reborn_home: &Path) -> anyhow::Result<bool> {
    Ok(
        read_token_file_checked(&webui_token_file_path(reborn_home))?
            .is_some_and(|contents| contents.trim().len() >= WEBUI_TOKEN_MIN_BYTES),
    )
}

/// Ensure `<reborn_home>/webui-token` holds a valid (>= entropy floor)
/// token, generating and writing one with `0600` permissions (unix) if
/// none exists yet.
///
/// Idempotent by design, independent of any `--force` flag: a valid
/// existing token is never regenerated, because operators may already
/// have long-lived sessions or an externally-copied env var keyed to
/// its current value. Only a missing or invalid (too-short) file is
/// (re)written; an unreadable-but-present file is a hard error here
/// (never silently treated as "missing, go ahead and overwrite it").
pub(crate) fn ensure_webui_token_file(reborn_home: &Path) -> anyhow::Result<FileWriteAction> {
    let file_path = webui_token_file_path(reborn_home);
    if webui_token_file_is_valid(reborn_home)? {
        repair_token_file_mode(&file_path)?;
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

/// Unconditionally replace `<reborn_home>/webui-token` with a freshly
/// generated token, regardless of whether the existing one is valid.
///
/// Distinct from [`ensure_webui_token_file`], which deliberately preserves
/// a valid existing token — `config set webui.token --rotate` is the one
/// caller that must invalidate every existing session on purpose (the
/// token doubles as the session-signing HMAC key, so rotating it kills
/// every live WebChat v2 session; the caller is responsible for warning
/// the operator before calling this).
pub(crate) fn rotate_webui_token_file(reborn_home: &Path) -> anyhow::Result<()> {
    let file_path = webui_token_file_path(reborn_home);
    let token = generate_webui_token();
    write_token_file(&file_path, &token)
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

/// Which of [`resolve_webui_token`]'s two sources produced the resolved
/// bearer token. `serve` only mounts the CLI-printed `/login?token=` route
/// when the token came from the file — an env-sourced token (e.g. a
/// Railway-shaped deployment) must never appear in that route's query
/// string, where it would leak into edge/proxy access logs. See
/// `commands::serve::execute`'s `cli_login_mount` and the
/// `onboard`/`status` login-link printers, which need the source to avoid
/// advertising a link to an unmounted route.
#[cfg(feature = "webui-v2-beta")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WebuiTokenSource {
    /// Resolved from the operator's env var (`[webui].env_token_var`,
    /// default `IRONCLAW_REBORN_WEBUI_TOKEN`).
    Env,
    /// Resolved from the onboarding-provisioned
    /// `<reborn_home>/webui-token` fallback file.
    File,
}

/// [`resolve_webui_token`]'s result: the bearer value plus which source
/// produced it (see [`WebuiTokenSource`]).
///
/// `Debug` is hand-written (not derived) to redact `value`: it doubles as
/// the WebChat v2 bearer credential *and* the session-signing HMAC key (see
/// the module doc), so a derived `Debug` would print the live secret
/// verbatim into any log line or panic message that formats this struct.
#[cfg(feature = "webui-v2-beta")]
pub(crate) struct ResolvedWebuiToken {
    pub(crate) value: String,
    pub(crate) source: WebuiTokenSource,
}

#[cfg(feature = "webui-v2-beta")]
impl std::fmt::Debug for ResolvedWebuiToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedWebuiToken")
            .field("value", &"<redacted>")
            .field("source", &self.source)
            .finish()
    }
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
) -> anyhow::Result<ResolvedWebuiToken> {
    if let Some(value) = env_value
        && !value.is_empty()
    {
        validate_token_entropy(value, env_var_name, reborn_home)?;
        return Ok(ResolvedWebuiToken {
            value: value.to_string(),
            source: WebuiTokenSource::Env,
        });
    }

    let file_path = webui_token_file_path(reborn_home);
    // `read_token_file_checked` rejects a symlinked or oversized file and
    // propagates real I/O errors instead of reading them as "absent" —
    // an unreadable token file must fail closed here, not silently fall
    // through to the "neither source found" error below.
    let file_value = read_token_file_checked(&file_path)?
        .map(|contents| contents.trim().to_string())
        .filter(|trimmed| !trimmed.is_empty());

    match file_value {
        Some(token) => {
            validate_token_entropy(&token, env_var_name, reborn_home)?;
            // Accepting this token as the live credential: repair a
            // wrongly-permissioned file in place (see
            // `repair_token_file_mode`'s doc for why repair, not reject).
            repair_token_file_mode(&file_path)?;
            Ok(ResolvedWebuiToken {
                value: token,
                source: WebuiTokenSource::File,
            })
        }
        None => Err(anyhow::anyhow!(
            "{env_var_name} must be set to the WebChat v2 bearer token, or a token file must \
             exist at {} (written by `ironclaw onboard`). Neither was found.",
            file_path.display()
        )),
    }
}

/// `true` when `serve` will source its webui bearer token from the env var
/// rather than the token file — the same precedence check
/// [`resolve_webui_token`] makes, exposed standalone so `onboard`/`status`
/// can decide whether printing a file-token link is even useful.
///
/// `Ok(false)` only for a genuinely unset/empty var. A present-but-not-UTF-8
/// value is `Err`, not `Ok(false)` — reuses
/// `commands::serve::present_unicode_env_var`'s unset-vs-not-unicode
/// distinction so `onboard`/`status` can't disagree with `serve` about
/// whether the var is "active".
#[cfg(feature = "webui-v2-beta")]
pub(crate) fn env_token_is_active(env_var_name: &str) -> anyhow::Result<bool> {
    Ok(
        crate::commands::serve::present_unicode_env_var(env_var_name)?
            .is_some_and(|value| !value.is_empty()),
    )
}

/// Resolve which env var name gates the webui bearer token: the operator's
/// `[webui].env_token_var` override when set, else
/// `commands::serve::DEFAULT_ENV_TOKEN_VAR`. Shared so `onboard`/`status`
/// check [`env_token_is_active`] against the same name `serve` resolves
/// against.
#[cfg(feature = "webui-v2-beta")]
pub(crate) fn resolve_env_token_var_name(
    config_file: Option<&ironclaw_reborn_config::RebornConfigFile>,
) -> &str {
    config_file
        .and_then(|file| file.webui.as_ref())
        .and_then(|section| section.env_token_var.as_deref())
        .unwrap_or(crate::commands::serve::DEFAULT_ENV_TOKEN_VAR)
}

/// What a login-link printer (`onboard`'s finale, `status`) should announce.
/// A file-token link is only useful — and only points at a route `serve`
/// actually mounts — when `serve` will source its bearer from the token file
/// rather than an env var (see [`WebuiTokenSource`]'s doc for why the CLI
/// login route is file-source-only).
#[cfg(feature = "webui-v2-beta")]
pub(crate) enum LoginLinkAnnouncement {
    /// The CLI-token login link, ready to print.
    Link(String),
    /// The env var is active; printing a file-token link would advertise a
    /// route `serve` won't mount. Carries the env var name so the caller can
    /// name it in the note.
    EnvTokenActive { env_var_name: String },
    /// Neither source is available yet (e.g. `onboard` hasn't provisioned
    /// the token file, or it's invalid).
    Unavailable,
}

/// Resolve what a login-link printer should announce — see
/// [`LoginLinkAnnouncement`]. Checks the env var first (matching
/// `resolve_webui_token`'s own precedence): an active env var always wins,
/// regardless of whether a valid token file also happens to exist.
///
/// Propagates a real error when the env var is set but not valid UTF-8 —
/// see [`env_token_is_active`] — rather than silently treating it as
/// inactive.
#[cfg(feature = "webui-v2-beta")]
pub(crate) fn resolve_login_link_announcement(
    home: &ironclaw_reborn_config::RebornHome,
    config_file: Option<&ironclaw_reborn_config::RebornConfigFile>,
) -> anyhow::Result<LoginLinkAnnouncement> {
    let env_var_name = resolve_env_token_var_name(config_file);
    if env_token_is_active(env_var_name)? {
        return Ok(LoginLinkAnnouncement::EnvTokenActive {
            env_var_name: env_var_name.to_string(),
        });
    }
    Ok(match login_link(home)? {
        Some(link) => LoginLinkAnnouncement::Link(link),
        None => LoginLinkAnnouncement::Unavailable,
    })
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
         `ironclaw onboard` to provision a valid token file.",
        webui_token_file_path(reborn_home).display(),
        value.len(),
    ))
}

/// The CLI-printed bootstrap link into the browser session — `Some` only
/// when a valid `webui-token` file is present (may not be true in
/// contexts like `status`, where onboarding may not have run). Uses
/// `serve`'s own default host:port constants. Shared by every caller that
/// prints a login link (`onboard`, `status`) so the construction lives in
/// one place.
#[cfg(feature = "webui-v2-beta")]
pub(crate) fn login_link(
    home: &ironclaw_reborn_config::RebornHome,
) -> anyhow::Result<Option<String>> {
    let token = read_token_file_checked(&webui_token_file_path(home.path()))?;
    Ok(token
        .filter(|contents| contents.trim().len() >= WEBUI_TOKEN_MIN_BYTES)
        .map(|contents| {
            format!(
                "http://{}:{}/login?token={}",
                crate::commands::serve::DEFAULT_SERVE_HOST,
                crate::commands::serve::DEFAULT_SERVE_PORT,
                contents.trim()
            )
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "webui-v2-beta")]
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

    #[test]
    fn rotate_webui_token_file_replaces_a_valid_existing_token() {
        let dir = tempfile::tempdir().expect("tempdir");
        ensure_webui_token_file(dir.path()).expect("seed valid token");
        let path = webui_token_file_path(dir.path());
        let before = fs::read_to_string(&path).expect("read token file");

        rotate_webui_token_file(dir.path()).expect("rotate must succeed");

        let after = fs::read_to_string(&path).expect("read rotated token file");
        assert_ne!(before, after, "rotate must generate a new token value");
        assert!(after.trim().len() >= WEBUI_TOKEN_MIN_BYTES);
    }

    #[test]
    fn rotate_webui_token_file_creates_a_token_when_none_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(!webui_token_file_is_valid(dir.path()).expect("query must succeed"));

        rotate_webui_token_file(dir.path()).expect("rotate must succeed");

        assert!(webui_token_file_is_valid(dir.path()).expect("query must succeed"));
    }

    #[cfg(feature = "webui-v2-beta")]
    #[test]
    fn resolve_prefers_env_value_when_set() {
        let dir = tempfile::tempdir().expect("tempdir");
        let resolved = resolve_webui_token("SOME_TOKEN_VAR", Some(VALID_TOKEN), dir.path())
            .expect("env value should resolve");
        assert_eq!(resolved.value, VALID_TOKEN);
        assert_eq!(resolved.source, WebuiTokenSource::Env);
    }

    #[cfg(feature = "webui-v2-beta")]
    #[test]
    fn resolve_falls_back_to_home_file_when_env_unset() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = webui_token_file_path(dir.path());
        fs::write(&path, format!("  {VALID_TOKEN}  \n")).expect("seed token file");

        let resolved = resolve_webui_token("SOME_TOKEN_VAR", None, dir.path())
            .expect("file fallback should resolve");
        assert_eq!(resolved.value, VALID_TOKEN, "file value must be trimmed");
        assert_eq!(resolved.source, WebuiTokenSource::File);
    }

    #[cfg(feature = "webui-v2-beta")]
    #[test]
    fn resolved_webui_token_debug_redacts_the_value() {
        let resolved = ResolvedWebuiToken {
            value: VALID_TOKEN.to_string(),
            source: WebuiTokenSource::Env,
        };
        let debug_output = format!("{resolved:?}");
        assert!(
            !debug_output.contains(VALID_TOKEN),
            "Debug output must not contain the bearer token verbatim: {debug_output}"
        );
        assert!(
            debug_output.contains("<redacted>"),
            "Debug output should mark the value as redacted: {debug_output}"
        );
        assert!(
            debug_output.contains("Env"),
            "Debug output should still show the (non-secret) source: {debug_output}"
        );
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

    // ── Hygiene: unreadable / symlinked / oversized token files ────

    #[test]
    fn ensure_webui_token_file_propagates_a_real_read_error_instead_of_overwriting() {
        // Before this fix, `webui_token_file_is_valid` mapped ANY read
        // error (not just "file absent") to `false` via `.unwrap_or(false)`,
        // so `ensure_webui_token_file` would silently overwrite an
        // existing-but-unreadable secret. A directory at the token path
        // reproduces "exists but unreadable as a file" without relying on
        // permission bits that root bypasses in CI.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = webui_token_file_path(dir.path());
        fs::create_dir_all(&path).expect("seed a directory at the token path");

        let error = ensure_webui_token_file(dir.path())
            .expect_err("a real I/O error reading the token path must propagate, not overwrite");
        assert!(
            error.to_string().contains(&path.display().to_string()),
            "error should name the token path: {error}"
        );
        assert!(path.is_dir(), "must not have written through the error");
    }

    #[test]
    fn webui_token_file_is_valid_propagates_a_real_read_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = webui_token_file_path(dir.path());
        fs::create_dir_all(&path).expect("seed a directory at the token path");

        webui_token_file_is_valid(dir.path())
            .expect_err("a directory at the token path is a real I/O error, not `Ok(false)`");
    }

    #[cfg(unix)]
    #[test]
    fn webui_token_file_is_valid_rejects_a_symlinked_token_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("elsewhere");
        fs::write(&target, "0".repeat(WEBUI_TOKEN_MIN_BYTES)).expect("write symlink target");
        let path = webui_token_file_path(dir.path());
        std::os::unix::fs::symlink(&target, &path).expect("create symlink");

        let error = webui_token_file_is_valid(dir.path())
            .expect_err("a symlinked token file must be rejected, not read through");
        assert!(error.to_string().contains("symlink"), "error: {error}");
    }

    #[cfg(unix)]
    #[test]
    fn ensure_webui_token_file_rejects_a_symlinked_token_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("elsewhere");
        fs::write(&target, "0".repeat(WEBUI_TOKEN_MIN_BYTES)).expect("write symlink target");
        let path = webui_token_file_path(dir.path());
        std::os::unix::fs::symlink(&target, &path).expect("create symlink");

        ensure_webui_token_file(dir.path())
            .expect_err("must refuse to read through, repair, or overwrite a symlinked token file");
    }

    #[cfg(unix)]
    #[test]
    fn ensure_webui_token_file_repairs_a_wrongly_permissioned_valid_token_in_place() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = webui_token_file_path(dir.path());
        let token = "0".repeat(WEBUI_TOKEN_MIN_BYTES);
        fs::write(&path, &token).expect("seed valid token file");
        fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("loosen permissions to 0644");

        let action = ensure_webui_token_file(dir.path())
            .expect("a valid token with a wrong mode must be repaired, not rejected");
        assert_eq!(
            action,
            FileWriteAction::Preserved,
            "repair-in-place must not regenerate the token value"
        );
        let contents = fs::read_to_string(&path).expect("read token file");
        assert_eq!(contents, token, "repair must not change the token content");
        let mode = fs::metadata(&path)
            .expect("stat token file")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "mode must be repaired to 0600, got {mode:o}");
    }

    #[cfg(unix)]
    #[test]
    fn webui_token_file_is_valid_rejects_a_fifo_without_blocking() {
        // Before the single-handle O_NONBLOCK|O_NOFOLLOW fix, a FIFO
        // planted at the token path would pass a standalone
        // `symlink_metadata().len()` check (FIFOs report length 0, well
        // under the cap) and then a blocking `read_to_string` open would
        // hang forever waiting for a writer — hanging `serve` startup
        // indefinitely. This must return promptly with a "not a regular
        // file" error instead.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = webui_token_file_path(dir.path());
        let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
            .expect("path has no interior NUL");
        // SAFETY: `mkfifo` with a valid NUL-terminated path and a
        // regular permission-bits argument; no aliasing/lifetime hazards.
        let rc = unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) };
        assert_eq!(rc, 0, "mkfifo failed: {}", std::io::Error::last_os_error());

        let error = webui_token_file_is_valid(dir.path()).expect_err(
            "a FIFO at the token path must be rejected, not read through or blocked on",
        );
        assert!(
            error.to_string().contains("not a regular file"),
            "error: {error}"
        );
    }

    #[test]
    fn ensure_webui_token_file_rejects_an_oversized_token_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = webui_token_file_path(dir.path());
        fs::write(&path, "0".repeat(WEBUI_TOKEN_FILE_MAX_BYTES as usize + 1))
            .expect("seed oversized token file");

        let error = ensure_webui_token_file(dir.path())
            .expect_err("an oversized token file must be rejected, not read unbounded");
        assert!(error.to_string().contains("bytes"), "error: {error}");
    }

    #[cfg(feature = "webui-v2-beta")]
    #[cfg(unix)]
    #[test]
    fn resolve_rejects_a_symlinked_token_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("elsewhere");
        fs::write(&target, VALID_TOKEN).expect("write symlink target");
        let path = webui_token_file_path(dir.path());
        std::os::unix::fs::symlink(&target, &path).expect("create symlink");

        let error = resolve_webui_token("SOME_TOKEN_VAR", None, dir.path())
            .expect_err("serve must refuse to authenticate off a symlinked token file");
        assert!(error.to_string().contains("symlink"), "error: {error}");
    }

    #[cfg(feature = "webui-v2-beta")]
    #[cfg(unix)]
    #[test]
    fn resolve_repairs_a_wrongly_permissioned_token_file_on_accept() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = webui_token_file_path(dir.path());
        fs::write(&path, VALID_TOKEN).expect("seed valid token file");
        fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("loosen permissions to 0644");

        let resolved = resolve_webui_token("SOME_TOKEN_VAR", None, dir.path())
            .expect("a valid token with a wrong mode must still be accepted");
        assert_eq!(resolved.value, VALID_TOKEN);
        let mode = fs::metadata(&path)
            .expect("stat token file")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "mode must be repaired to 0600, got {mode:o}");
    }

    #[cfg(feature = "webui-v2-beta")]
    #[test]
    fn env_token_is_active_true_when_env_var_set_and_non_empty() {
        let _guard = crate::runtime::test_env::lock_runtime_env();
        const VAR: &str = "IRONCLAW_REBORN_CLI_TEST_TOKEN_SOURCE_ACTIVE_VAR";
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var(VAR, "some-token-value") };
        let active = env_token_is_active(VAR);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe { std::env::remove_var(VAR) };
        assert!(
            active.expect("a present unicode var is not an error"),
            "a non-empty env var must count as active"
        );
    }

    #[cfg(feature = "webui-v2-beta")]
    #[test]
    fn env_token_is_active_false_when_unset_or_empty() {
        let _guard = crate::runtime::test_env::lock_runtime_env();
        const VAR: &str = "IRONCLAW_REBORN_CLI_TEST_TOKEN_SOURCE_INACTIVE_VAR";
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe { std::env::remove_var(VAR) };
        assert!(
            !env_token_is_active(VAR).expect("unset is not an error"),
            "an unset env var is not active"
        );

        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var(VAR, "") };
        let active_when_empty = env_token_is_active(VAR);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe { std::env::remove_var(VAR) };
        assert!(
            !active_when_empty.expect("a present empty unicode var is not an error"),
            "an empty-string env var must not count as active, matching \
             `resolve_webui_token`'s own non-empty check"
        );
    }

    #[cfg(feature = "webui-v2-beta")]
    #[cfg(unix)]
    #[test]
    fn env_token_is_active_propagates_not_unicode_instead_of_treating_it_as_inactive() {
        // Mirrors `commands::serve::present_unicode_env_var_propagates_not_unicode_instead_of_treating_it_as_unset`:
        // a mangled-UTF-8 token env var must not collapse to "inactive"
        // here while `serve` itself fails closed on the same value.
        use std::os::unix::ffi::OsStringExt as _;

        let _guard = crate::runtime::test_env::lock_runtime_env();
        const VAR: &str = "IRONCLAW_REBORN_CLI_TEST_TOKEN_SOURCE_NON_UNICODE_VAR";
        let invalid_utf8 = std::ffi::OsString::from_vec(vec![0xFF, 0xFE, 0xFD]);
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var(VAR, &invalid_utf8) };
        let result = env_token_is_active(VAR);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe { std::env::remove_var(VAR) };

        let error = result.expect_err("non-UTF-8 env value must be a real error, not `Ok(false)`");
        assert!(
            error.to_string().contains(VAR),
            "error should name the var: {error}"
        );
    }
}
