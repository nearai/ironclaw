//! Sidecar lifecycle: configuration, the per-boot token, and supervision
//! (attested-signing §E2).
//!
//! ## Deployment shapes
//!
//! `ATTESTED_ALPACA_SIDECAR` selects one of three, and an unparseable value is
//! a hard startup error rather than a silent fallback — a signing deployment
//! must never quietly run in a different mode than the operator asked for:
//!
//! * `off` — no sidecar. Chains that need it fail closed with a configuration
//!   error ([`UnconfiguredAlpacaPort`]); chains `ironclaw_chain_signing` covers
//!   natively are unaffected because they never reach the port.
//! * `managed` — this process spawns and supervises the child.
//! * `external:<socket-path>` — an already-running sidecar (dev, or an operator
//!   running it under their own supervisor). We connect but do not manage.
//!
//! ## The token
//!
//! Generated here, per boot, and handed to the child on **stdin** — never argv
//! (visible in `ps`), never the environment (visible in a crash dump or a
//! child's `/proc`). It is not a security boundary on its own: the socket's
//! `0700` directory is. The token defends against a same-user process that
//! finds the socket path.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use crate::alpaca::{AlpacaError, SharedAlpacaPort};

/// How the sidecar is deployed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlpacaDeployment {
    /// No sidecar; sidecar-dependent chains fail closed.
    Off,
    /// This process spawns and supervises the child.
    Managed {
        /// Where the socket will be created.
        socket_path: PathBuf,
    },
    /// Connect to a sidecar someone else runs.
    External {
        /// The existing socket.
        socket_path: PathBuf,
    },
}

/// Why a sidecar configuration was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AlpacaConfigError {
    /// The value was not one of the three recognized shapes.
    #[error("ATTESTED_ALPACA_SIDECAR must be `off`, `managed`, or `external:<socket-path>`")]
    Unrecognized,

    /// `external:` was given with no path.
    #[error("ATTESTED_ALPACA_SIDECAR=external: requires a socket path")]
    MissingPath,

    /// The socket path exceeds what the OS allows for a unix socket.
    ///
    /// Caught at configuration time because the alternative is a bare `EINVAL`
    /// from `listen`/`connect` much later, which is genuinely hard to diagnose
    /// in a supervised child.
    #[error("socket path is {actual} bytes; the unix socket limit is {limit}")]
    SocketPathTooLong {
        /// The offending length.
        actual: usize,
        /// The platform limit applied.
        limit: usize,
    },
}

/// `sun_path` is 104 bytes on macOS/BSD and 108 on Linux; use the smaller so a
/// configuration that works on one developer's machine works on all of them.
pub const SOCKET_PATH_MAX: usize = 104;

impl AlpacaDeployment {
    /// Parse the `ATTESTED_ALPACA_SIDECAR` value.
    ///
    /// `managed` takes its socket path from `default_socket_path`, which the
    /// caller derives from the runtime's own directory layout.
    pub fn parse(value: &str, default_socket_path: &str) -> Result<Self, AlpacaConfigError> {
        let value = value.trim();
        let deployment = match value {
            "off" => Self::Off,
            "managed" => Self::Managed {
                socket_path: PathBuf::from(default_socket_path),
            },
            other => {
                let path = other
                    .strip_prefix("external:")
                    .ok_or(AlpacaConfigError::Unrecognized)?;
                if path.is_empty() {
                    return Err(AlpacaConfigError::MissingPath);
                }
                Self::External {
                    socket_path: PathBuf::from(path),
                }
            }
        };
        deployment.validate()?;
        Ok(deployment)
    }

    fn validate(&self) -> Result<(), AlpacaConfigError> {
        let path = match self {
            Self::Off => return Ok(()),
            Self::Managed { socket_path } | Self::External { socket_path } => socket_path,
        };
        let actual = path.as_os_str().len();
        if actual > SOCKET_PATH_MAX {
            return Err(AlpacaConfigError::SocketPathTooLong {
                actual,
                limit: SOCKET_PATH_MAX,
            });
        }
        Ok(())
    }

    /// The socket this deployment talks to, if any.
    pub fn socket_path(&self) -> Option<&PathBuf> {
        match self {
            Self::Off => None,
            Self::Managed { socket_path } | Self::External { socket_path } => Some(socket_path),
        }
    }

    /// Whether this process is responsible for the child's lifecycle.
    pub fn is_managed(&self) -> bool {
        matches!(self, Self::Managed { .. })
    }
}

/// Restart pacing for a managed child.
///
/// Exponential with a ceiling: a sidecar that crashes on startup (bad config, a
/// missing Node) must not become a spawn loop that buries the real error in log
/// noise. The ceiling keeps recovery bounded once the cause is fixed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestartBackoff {
    /// Delay before the first restart.
    pub initial: Duration,
    /// Upper bound on the delay.
    pub max: Duration,
    /// How long a child must survive before its run counts as healthy and the
    /// escalation resets.
    ///
    /// Without this, a process that runs fine for a week and then dies once
    /// would be restarted at the ceiling, as if it had been crash-looping all
    /// along.
    pub reset_after: Duration,
}

impl Default for RestartBackoff {
    fn default() -> Self {
        Self {
            initial: Duration::from_millis(500),
            max: Duration::from_secs(30),
            reset_after: Duration::from_secs(60),
        }
    }
}

impl RestartBackoff {
    /// The delay before restart attempt `attempt` (0-based).
    pub fn delay_for(&self, attempt: u32) -> Duration {
        let doubled = self
            .initial
            .checked_mul(2u32.saturating_pow(attempt.min(16)))
            .unwrap_or(self.max);
        doubled.min(self.max)
    }
}

/// Mint a per-boot token.
///
/// 256 bits of OS entropy, hex. Regenerated every boot so a token recovered
/// from a core dump or a stale log is useless against the next run.
pub fn mint_sidecar_token() -> Result<String, AlpacaError> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|error| AlpacaError::Unavailable {
        reason: format!("entropy unavailable for the sidecar token: {error}"),
    })?;
    Ok(hex::encode(bytes))
}

/// Build the port for a deployment.
///
/// `Off` yields the fail-closed port rather than `None`, so callers have one
/// uniform shape and cannot accidentally treat "no sidecar" as "skip the
/// check".
pub fn port_for(deployment: &AlpacaDeployment, token: &str) -> SharedAlpacaPort {
    match deployment {
        AlpacaDeployment::Off => std::sync::Arc::new(crate::alpaca::UnconfiguredAlpacaPort),
        AlpacaDeployment::Managed { socket_path } | AlpacaDeployment::External { socket_path } => {
            std::sync::Arc::new(crate::alpaca_uds::UdsAlpacaPort::new(socket_path, token))
        }
    }
}

/// How to start the sidecar child.
///
/// The caller supplies the interpreter and script path rather than this crate
/// guessing at a Node location: composition knows the deployment's layout, and
/// a supervisor that searched `PATH` for `node` would pick up whatever a shell
/// profile happened to put there.
#[derive(Debug, Clone)]
pub struct SidecarSpawnSpec {
    /// The program to execute.
    pub program: OsString,
    /// Arguments. The token is never among them — it goes over stdin.
    pub args: Vec<OsString>,
    /// Environment entries to add (socket path, chain configuration).
    pub env: Vec<(OsString, OsString)>,
    /// Restart pacing.
    pub backoff: RestartBackoff,
    /// How long to let the child exit on its own after the stdin link closes,
    /// before killing it.
    pub shutdown_grace: Duration,
}

/// A running managed sidecar.
///
/// Owns the child's whole life: spawn, the stdin liveness link, restart on
/// crash, and reaping on shutdown. Dropping it kills the child (via
/// `kill_on_drop`), so a supervisor that goes out of scope during a panic
/// cannot leave an orphan holding the signing socket — but prefer
/// [`AlpacaSupervisor::shutdown`], which lets the child close its socket
/// cleanly first.
#[derive(Debug)]
pub struct AlpacaSupervisor {
    shutdown: tokio::sync::watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl AlpacaSupervisor {
    /// Start supervising a child.
    ///
    /// Returns immediately; the first spawn happens on the supervision task.
    /// Readiness is observed through the port's health check, not by blocking
    /// here — a sidecar that is slow to bind must not stall startup for
    /// everything else.
    pub fn spawn(spec: SidecarSpawnSpec, token: String) -> Self {
        let (shutdown, receiver) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(supervise(spec, token, receiver));
        Self { shutdown, task }
    }

    /// Stop the child and wait for it to be reaped.
    pub async fn shutdown(self) {
        // A receive error means the task already ended; nothing left to stop.
        let _ = self.shutdown.send(true);
        let _ = self.task.await;
    }
}

async fn supervise(
    spec: SidecarSpawnSpec,
    token: String,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let mut attempt = 0u32;

    loop {
        if *shutdown.borrow() {
            return;
        }

        let started = tokio::time::Instant::now();
        match spawn_child(&spec, &token).await {
            Ok((mut child, stdin)) => {
                // `stdin` is deliberately held across the wait: the open pipe
                // is the child's parent-liveness link, and `Child::wait` would
                // have dropped it had we left it inside the child.
                let stdin = Some(stdin);
                tokio::select! {
                    status = child.wait() => {
                        match status {
                            Ok(status) => tracing::warn!(
                                target: "ironclaw::attested::alpaca",
                                %status,
                                "alpaca sidecar exited; restarting"
                            ),
                            Err(error) => tracing::warn!(
                                target: "ironclaw::attested::alpaca",
                                %error,
                                "failed to await the alpaca sidecar; restarting"
                            ),
                        }
                    }
                    _ = shutdown.changed() => {
                        stop_child(&spec, child, stdin).await;
                        return;
                    }
                }

                if started.elapsed() >= spec.backoff.reset_after {
                    // The run was long enough to count as healthy, so this is a
                    // fresh failure rather than a continuing crash loop.
                    attempt = 0;
                }
            }
            Err(error) => {
                tracing::warn!(
                    target: "ironclaw::attested::alpaca",
                    %error,
                    program = ?spec.program,
                    "failed to spawn the alpaca sidecar; retrying"
                );
            }
        }

        let delay = spec.backoff.delay_for(attempt);
        attempt = attempt.saturating_add(1);
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = shutdown.changed() => return,
        }
    }
}

async fn spawn_child(
    spec: &SidecarSpawnSpec,
    token: &str,
) -> std::io::Result<(tokio::process::Child, tokio::process::ChildStdin)> {
    use tokio::io::AsyncWriteExt as _;

    let mut command = tokio::process::Command::new(&spec.program);
    command
        .args(&spec.args)
        .stdin(std::process::Stdio::piped())
        // A panicking or aborted parent must not leave the child holding the
        // signing socket.
        .kill_on_drop(true);
    for (key, value) in &spec.env {
        command.env(key, value);
    }

    let mut child = command.spawn()?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| std::io::Error::other("the sidecar child was spawned without stdin"))?;

    // Newline-terminated: the child reads one line and then treats the still-open
    // pipe as the liveness link.
    stdin.write_all(token.as_bytes()).await?;
    stdin.write_all(b"\n").await?;
    stdin.flush().await?;

    Ok((child, stdin))
}

async fn stop_child(
    spec: &SidecarSpawnSpec,
    mut child: tokio::process::Child,
    stdin: Option<tokio::process::ChildStdin>,
) {
    // Closing the link is the graceful signal: the child unlinks its socket and
    // exits on its own.
    drop(stdin);

    if tokio::time::timeout(spec.shutdown_grace, child.wait())
        .await
        .is_ok()
    {
        return;
    }

    // It ignored the link, so it does not get a say.
    let _ = child.start_kill();
    let _ = child.wait().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::ffi::OsString;
    use std::path::Path;

    const DEFAULT_SOCK: &str = "/tmp/ic/alpaca.sock";

    /// A child scripted with `sh` — the supervisor's contract is about process
    /// lifecycle, not about Node, so tests need no toolchain.
    fn scripted(script: &str) -> SidecarSpawnSpec {
        SidecarSpawnSpec {
            program: OsString::from("/bin/sh"),
            args: vec![OsString::from("-c"), OsString::from(script)],
            env: Vec::new(),
            backoff: RestartBackoff {
                initial: Duration::from_millis(10),
                max: Duration::from_millis(20),
                reset_after: Duration::from_secs(60),
            },
            shutdown_grace: Duration::from_millis(200),
        }
    }

    fn scratch(name: &str) -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let unique = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("ic-alpaca-sup-{name}-{unique}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    async fn wait_for(path: &Path, predicate: impl Fn(&str) -> bool) -> String {
        for _ in 0..400 {
            if let Ok(contents) = std::fs::read_to_string(path)
                && predicate(&contents)
            {
                return contents;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("timed out waiting on {}", path.display());
    }

    /// The token must arrive on stdin — not argv, where `ps` would show it to
    /// every user on the box, and not the environment.
    #[tokio::test]
    async fn the_token_reaches_the_child_over_stdin() {
        let dir = scratch("token");
        let out = dir.join("token.txt");
        let supervisor = AlpacaSupervisor::spawn(
            scripted(&format!("head -n 1 > {}; sleep 30", out.display())),
            "s3cr3t-token".to_string(),
        );

        let seen = wait_for(&out, |c| c.contains('\n') || !c.is_empty()).await;
        assert_eq!(seen.trim(), "s3cr3t-token");

        supervisor.shutdown().await;
    }

    /// A child that dies must come back. Without this the first transient
    /// crash silently disables signing for the rest of the process's life.
    #[tokio::test]
    async fn a_crashed_child_is_restarted() {
        let dir = scratch("restart");
        let log = dir.join("boots.txt");
        let supervisor = AlpacaSupervisor::spawn(
            scripted(&format!("echo boot >> {}; exit 1", log.display())),
            "token".to_string(),
        );

        let seen = wait_for(&log, |c| c.lines().count() >= 3).await;
        assert!(
            seen.lines().count() >= 3,
            "expected repeated restarts, saw: {seen:?}"
        );

        supervisor.shutdown().await;
    }

    /// Shutdown must actually reap the child. A surviving orphan would still
    /// hold the signing socket and outlive the process that vouched for it.
    #[tokio::test]
    async fn shutdown_reaps_the_child() {
        let dir = scratch("reap");
        let pidfile = dir.join("pid.txt");
        let supervisor = AlpacaSupervisor::spawn(
            scripted(&format!("echo $$ > {}; sleep 60", pidfile.display())),
            "token".to_string(),
        );

        let pid: i32 = wait_for(&pidfile, |c| !c.trim().is_empty())
            .await
            .trim()
            .parse()
            .expect("pid");
        supervisor.shutdown().await;

        // `kill -0` probes for existence without signalling.
        let alive = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .expect("kill -0")
            .success();
        assert!(!alive, "child {pid} survived shutdown");
    }

    /// The child holds stdin as its liveness link, so the supervisor must keep
    /// the write end open for the child's whole life — closing it early would
    /// read to the child as "the parent died" and trigger a shutdown loop.
    #[tokio::test]
    async fn the_stdin_link_stays_open_after_the_token() {
        let dir = scratch("link");
        let out = dir.join("eof.txt");
        // `cat` only reaches EOF — and so only writes the marker — once our
        // write end closes.
        let supervisor = AlpacaSupervisor::spawn(
            scripted(&format!("cat > /dev/null; echo eof > {}", out.display())),
            "token".to_string(),
        );

        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            !out.exists(),
            "the child saw EOF while the supervisor was still running"
        );

        supervisor.shutdown().await;
        let seen = wait_for(&out, |c| c.contains("eof")).await;
        assert!(seen.contains("eof"), "shutdown must close the stdin link");
    }

    #[test]
    fn the_three_deployment_shapes_parse() {
        assert_eq!(
            AlpacaDeployment::parse("off", DEFAULT_SOCK),
            Ok(AlpacaDeployment::Off)
        );
        assert_eq!(
            AlpacaDeployment::parse("managed", DEFAULT_SOCK),
            Ok(AlpacaDeployment::Managed {
                socket_path: PathBuf::from(DEFAULT_SOCK)
            })
        );
        assert_eq!(
            AlpacaDeployment::parse("external:/tmp/theirs.sock", DEFAULT_SOCK),
            Ok(AlpacaDeployment::External {
                socket_path: PathBuf::from("/tmp/theirs.sock")
            })
        );
        // Surrounding whitespace from a config file is not an error.
        assert_eq!(
            AlpacaDeployment::parse("  off  ", DEFAULT_SOCK),
            Ok(AlpacaDeployment::Off)
        );
    }

    /// An unrecognized value is a hard error, never a silent fallback: a
    /// signing deployment must not quietly run in a mode nobody asked for.
    #[test]
    fn an_unrecognized_value_is_refused_rather_than_defaulted() {
        for value in ["", "on", "enabled", "true", "external"] {
            assert!(
                AlpacaDeployment::parse(value, DEFAULT_SOCK).is_err(),
                "{value:?} must not parse to a working deployment"
            );
        }
        assert_eq!(
            AlpacaDeployment::parse("external:", DEFAULT_SOCK),
            Err(AlpacaConfigError::MissingPath)
        );
    }

    /// Caught here rather than as a bare `EINVAL` from the socket syscall much
    /// later — the failure mode the sidecar's own startup guard also covers.
    #[test]
    fn an_over_long_socket_path_is_refused_at_configuration_time() {
        let long = format!("/tmp/{}/alpaca.sock", "x".repeat(120));
        assert!(matches!(
            AlpacaDeployment::parse(&format!("external:{long}"), DEFAULT_SOCK),
            Err(AlpacaConfigError::SocketPathTooLong { .. })
        ));
        // `off` has no socket, so length never applies.
        assert_eq!(
            AlpacaDeployment::parse("off", &long),
            Ok(AlpacaDeployment::Off)
        );
    }

    #[test]
    fn only_the_managed_shape_owns_the_child_lifecycle() {
        assert!(
            AlpacaDeployment::parse("managed", DEFAULT_SOCK)
                .unwrap()
                .is_managed()
        );
        // External means someone else supervises it — we must not kill or
        // restart a process we did not start.
        assert!(
            !AlpacaDeployment::parse("external:/tmp/x.sock", DEFAULT_SOCK)
                .unwrap()
                .is_managed()
        );
        assert!(!AlpacaDeployment::Off.is_managed());
    }

    #[test]
    fn backoff_grows_then_holds_at_the_ceiling() {
        let backoff = RestartBackoff::default();
        assert_eq!(backoff.delay_for(0), Duration::from_millis(500));
        assert_eq!(backoff.delay_for(1), Duration::from_secs(1));
        assert_eq!(backoff.delay_for(2), Duration::from_secs(2));
        // A crash-looping child must not become a spawn storm.
        assert_eq!(backoff.delay_for(20), backoff.max);
        assert_eq!(backoff.delay_for(u32::MAX), backoff.max);
    }

    #[test]
    fn tokens_are_long_and_never_repeat() {
        let first = mint_sidecar_token().expect("entropy");
        let second = mint_sidecar_token().expect("entropy");
        assert_eq!(first.len(), 64, "256 bits, hex");
        assert_ne!(first, second, "a token is per-boot, not a constant");
    }

    /// `Off` must still yield a port — a `None` would invite a caller to treat
    /// "no sidecar" as "skip the sidecar step" instead of failing closed.
    #[tokio::test]
    async fn the_off_deployment_yields_a_fail_closed_port() {
        use crate::alpaca::{CraftRequest, CurrencyId};
        let port = port_for(&AlpacaDeployment::Off, "token");
        assert!(!port.healthy().await);
        assert!(matches!(
            port.craft_transaction(CraftRequest {
                currency_id: CurrencyId::new("ethereum_sepolia"),
                params: serde_json::Value::Null,
            })
            .await,
            Err(AlpacaError::Unavailable { .. })
        ));
    }

    #[tokio::test]
    async fn a_configured_deployment_yields_a_socket_port() {
        let port = port_for(
            &AlpacaDeployment::External {
                socket_path: PathBuf::from("/tmp/definitely-absent-ic.sock"),
            },
            "token",
        );
        // Nothing is listening, so it must report unhealthy rather than error.
        assert!(!port.healthy().await);
    }
}
