//! Bootstrap `.env` loading, performed once before any command runs.
//!
//! Bootstrap variables (`DATABASE_URL`, `LLM_BACKEND`, and every `api_key_env`
//! a `config.toml` slot names) have to be readable before the runtime exists to
//! read them from anywhere else, so they come from the process environment and
//! the `.env` files that seed it.

use ironclaw_config::RebornHome;

/// Load the bootstrap `.env` files into the process environment.
///
/// Precedence, strongest first. `dotenvy` never overwrites a variable that is
/// already set, so the first loader to define a key wins:
///
/// 1. the real process environment (shell export, systemd unit, launchd plist)
/// 2. `./.env`, searched upward from the working directory, for dev checkouts
/// 3. `$IRONCLAW_REBORN_HOME/.env`, the operator file that sits beside
///    `config.toml` and `providers.json`
///
/// (3) is resolved *after* (2) so a checkout may point the binary at a
/// different home and have that redirection respected.
///
/// Only (2) used to be loaded. A host started from anywhere but a checkout
/// carrying a `.env` therefore booted with none of its bootstrap variables,
/// even though its own Reborn home held them: `[llm.default]` with
/// `api_key_env = "NEARAI_API_KEY"` resolved to no key at all, and the nearai
/// provider fell through to a session-token path that no `SessionRenewer` is
/// wired to renew at `serve` boot. The run then failed advising the operator to
/// check an API key that had never been read.
pub(crate) fn load() {
    // Silent on a missing file — production hosts use shell-exported env or a
    // unit file, not `.env` — but any other error (parse failure, permission
    // denied) is surfaced to stderr so a malformed file does not boot the host
    // with stale env. Boot still proceeds, because operators may have already
    // exported the same keys in their shell.
    if let Err(error) = dotenvy::dotenv()
        && !error.not_found()
    {
        eprintln!("warning: failed to load .env: {error}");
    }

    // silent-ok: a home that does not resolve simply has no operator `.env` to
    // load. Every command that needs the home resolves it again through
    // `RebornCliContext` and reports the failure there with full context, so
    // swallowing it here cannot hide it.
    let Ok(home) = RebornHome::resolve_from_env() else {
        return;
    };

    let path = home.path().join(".env");
    if let Err(error) = dotenvy::from_path(&path)
        && !error.not_found()
    {
        eprintln!("warning: failed to load {}: {error}", path.display());
    }
}
