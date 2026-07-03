//! `RebornCapabilityBackend` — the capability-backend selector and its
//! `install` method, extracted from `builder.rs`'s `build()` to keep that
//! file under the repo's 1000-line file-size guardrail.

use std::sync::Arc;

use super::group::GroupCapability;
use super::harness::HostRuntimeCapabilityHarness;
use super::http_matcher::ScriptedHttpResponse;
use super::process::ScriptedProcessResult;

type HarnessResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Provider id prefix used by every mock-MCP test capability and assertion.
/// One owner for the string — the `MockMcp` variant and `assert_mcp_tool_called`
/// both derive their ids from this constant.
pub(super) const MOCK_MCP_PROVIDER_ID: &str = "mock-mcp";

/// Selects the capability backend the integration harness wires.
pub(super) enum RebornCapabilityBackend {
    /// Echo recorder: records capability invocations, executes nothing. Default —
    /// a text-only turn invokes no tool.
    Echo,
    /// Real first-party tool runtime (`builtin.http` + friends) with the recording
    /// `RuntimeHttpEgress` (scripted body, no network) — the §3.7 Tier-2 capture.
    BuiltinHttpTools,
    /// Real MCP runtime wired to a loopback mock MCP server (slice 6 §3.6).
    /// Uses `LoopbackMcpRuntimeHttpEgress` which makes real HTTP connections to
    /// the mock server; no real credentials or network policy are required.
    MockMcp { mcp_url: String },
    /// GitHub first-party WASM capabilities with a `GithubHarnessAuthorizer`
    /// that attaches an `InjectCredentialAccountOnce` obligation, so a dispatched
    /// `github.*` tool call gets a synthetic access token injected onto the
    /// outbound request (T0-SECRET-INJECT). The credential lands on the recorded
    /// **network** egress (`assert_network_egress_header_contains`); the runtime
    /// egress recorder is inert for this wiring — see that assertion's docs.
    GithubIssueTools,
    /// Real first-party `web-access.search` / `web-access.get_content`
    /// capabilities (C-WEBACCESS), dispatched through the real
    /// `WebAccessExecutor` via the production
    /// `register_bundled_web_access_first_party_handlers` registration.
    /// web-access declares no `runtime_credentials`, so this wires the plain default
    /// `GrantAuthorizer` — no credential-injecting authorizer is needed.
    WebAccessTools,
}

/// Which process port the built `BuiltinHttpTools` runtime installs for
/// `builtin.shell`. These are mutually exclusive; the builder holds exactly one.
#[derive(Debug, Clone, Default)]
pub(super) enum ShellMode {
    /// Slice 5 default: the inert `RecordingProcessPort` records the command and
    /// spawns no OS process.
    #[default]
    Inert,
    /// The real `LocalHostProcessPort` runs a (hermetic) command for real.
    Live,
    /// The inert recording port returns a scripted result (error-path coverage):
    /// a non-zero exit code or a `run_command` error.
    Scripted(ScriptedProcessResult),
}

impl RebornCapabilityBackend {
    /// Install this capability backend, producing the `GroupCapability` the
    /// harness's group/thread builder wires. Echo by default (records, executes
    /// nothing — a text reply invokes no tool). Builtin/MCP swap in the real
    /// first-party runtime. (Live approval stores are a group-only backend; see
    /// `RebornIntegrationGroup::live_approvals`.)
    pub(super) async fn install(
        self,
        shell_mode: ShellMode,
        keyed_http_responses: Vec<ScriptedHttpResponse>,
        web_access_response_bodies: Vec<Vec<u8>>,
        github_network_statuses: Vec<u16>,
    ) -> HarnessResult<GroupCapability> {
        Ok(match self {
            RebornCapabilityBackend::Echo => GroupCapability::Recording,
            RebornCapabilityBackend::BuiltinHttpTools => {
                // Slice 5: `.with_live_shell()` opts into the real LocalHostProcessPort;
                // `Inert`/`Scripted` both use the inert RecordingProcessPort (the
                // latter with a canned result installed below).
                let host_runtime = match shell_mode {
                    ShellMode::Live => {
                        HostRuntimeCapabilityHarness::core_builtin_tools_with_live_shell().await?
                    }
                    ShellMode::Inert | ShellMode::Scripted(_) => {
                        HostRuntimeCapabilityHarness::core_builtin_tools().await?
                    }
                };
                host_runtime.install_http_responses(keyed_http_responses)?;
                if let ShellMode::Scripted(scripted_process) = shell_mode {
                    host_runtime.install_process_script(scripted_process)?;
                }
                GroupCapability::HostRuntime(Arc::new(host_runtime))
            }
            RebornCapabilityBackend::MockMcp { mcp_url } => {
                let host_runtime = HostRuntimeCapabilityHarness::mock_mcp_tools(
                    &mcp_url,
                    MOCK_MCP_PROVIDER_ID,
                    &format!("{MOCK_MCP_PROVIDER_ID}.search"),
                )
                .await?;
                GroupCapability::HostRuntime(Arc::new(host_runtime))
            }
            RebornCapabilityBackend::GithubIssueTools => {
                // T0-SECRET-INJECT (see the `GithubIssueTools` variant docs above):
                // no approval gate / user alignment — the authorizer allows every
                // dispatch outright.
                let host_runtime = HostRuntimeCapabilityHarness::github_issue_tools().await?;
                // W4-AUTHGATE-WIRE: wire keyed HTTP responses onto this backend too
                // (previously only `BuiltinHttpTools` installed them). A no-op for
                // existing callers that never populate `keyed_http_responses` for
                // this backend.
                host_runtime.install_http_responses(keyed_http_responses)?;
                // The real github WASM HTTP call flows through the **network**
                // egress lane, not the runtime-egress lane the matcher above
                // scripts (`try_with_host_http_egress` overwrites the runtime
                // port — see `reborn_integration_secret_injection.rs`'s module
                // doc), so a runtime-401-after-injection scenario scripts the
                // status here instead. A no-op (empty vec) for existing callers.
                for status in github_network_statuses {
                    host_runtime.install_network_status_script(status)?;
                }
                GroupCapability::HostRuntime(Arc::new(host_runtime))
            }
            RebornCapabilityBackend::WebAccessTools => {
                // C-WEBACCESS — see the `WebAccessTools` variant docs above.
                let host_runtime = HostRuntimeCapabilityHarness::web_access_tools().await?;
                host_runtime.install_web_access_responses(web_access_response_bodies)?;
                GroupCapability::HostRuntime(Arc::new(host_runtime))
            }
        })
    }
}
