//! Private pipe setup for an already enrolled local MCP server. No token is
//! accepted from arguments, environment, terminal, or a regular file.
use anyhow::{Context, anyhow, bail};
use clap::Args;
use ironclaw_auth::{
    AuthContinuationRef, AuthProductError, AuthProductScope, AuthProviderId, AuthSurface,
    CredentialAccountId, CredentialAccountLabel, CredentialAccountSelectionRequest,
    CredentialAccountUpdateBinding, RebornManualTokenSetupRequest, RebornManualTokenSubmitRequest,
    RuntimeCredentialAccountSelectionRequest,
};
use ironclaw_composition::{RebornRuntime, RebornRuntimeInput, build_reborn_runtime};
use ironclaw_extension_contracts::recipe::McpServerEndpoint;
use ironclaw_extension_manager::extension_lifecycle_command::{
    RebornExtensionLifecycleCommand, RebornExtensionLifecycleRuntime,
    execute_reborn_extension_lifecycle_command,
};
use ironclaw_host_api::{
    capability::{RuntimeCredentialAccountSetup, RuntimeCredentialRequirementSource},
    ids::{ExtensionId, InvocationId, SecretHandle},
    resource::ResourceScope,
};
use secrecy::SecretString;
use serde::Serialize;

use crate::context::RebornCliContext;
use crate::runtime::{RuntimeInputCaller, RuntimeInputOptions};

#[derive(Debug, Args)]
pub(crate) struct McpTokenImportCommand {
    /// Already enrolled and installed local MCP extension id.
    extension_id: String,
    /// Inherited private pipe descriptor; never a terminal or regular file.
    #[arg(long, required_unless_present = "inspect_account")]
    token_fd: Option<i32>,
    /// Inspect the selected public account reference without importing or activating.
    #[arg(long, conflicts_with_all = ["token_fd", "expected_account"])]
    inspect_account: bool,
    /// Public account id required when updating a previously selected account.
    #[arg(long)]
    expected_account: Option<uuid::Uuid>,
}

#[derive(Serialize)]
struct ImportReceipt {
    extension_id: ExtensionId,
    account_id: CredentialAccountId,
    handle: SecretHandle,
    endpoint: String,
    active: bool,
}

#[derive(Serialize)]
struct AccountInspection {
    kind: &'static str,
    extension_id: ExtensionId,
    account_id: Option<CredentialAccountId>,
    handle: SecretHandle,
    endpoint: String,
}

#[derive(Serialize)]
#[serde(untagged)]
enum SetupReceipt {
    Imported(ImportReceipt),
    Inspected(AccountInspection),
}

/// The native service deliberately returns a sanitized DTO, not a raw error.
/// Retain both public code and retryability when adapting to the CLI error chain.
#[derive(Debug, thiserror::Error)]
#[error("native MCP authentication failed: {0:?}")]
struct NativeAuthSetupError(ironclaw_auth::RebornAuthProductError);

impl McpTokenImportCommand {
    #[cfg(unix)]
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        let pipe = if self.inspect_account {
            None
        } else {
            Some(checked_pipe(self.token_fd.ok_or_else(|| {
                anyhow!("private token pipe descriptor is required")
            })?)?)
        };
        let extension_id = ExtensionId::new(self.extension_id).context("invalid extension id")?;
        let input = crate::runtime::build_services_input_with_options(
            context.boot_config(),
            RuntimeInputCaller::Run,
            RuntimeInputOptions {
                confirm_host_access: false,
            },
        )?;
        let executor = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        let receipt = executor.block_on(async move {
            let services =
                crate::runtime::with_binary_host_extension_bindings(input.services_input)?;
            let runtime =
                build_reborn_runtime(RebornRuntimeInput::from_build_input(services)).await?;
            let result =
                import_from_pipe(&runtime, extension_id, pipe, self.expected_account).await;
            runtime
                .shutdown()
                .await
                .context("private MCP setup shutdown failed")?;
            result
        })?;
        println!("{}", serde_json::to_string(&receipt)?);
        Ok(())
    }

    #[cfg(not(unix))]
    pub(crate) fn execute(self, _context: RebornCliContext) -> anyhow::Result<()> {
        bail!("private MCP pipe import is unsupported on this platform")
    }
}

#[cfg(unix)]
fn checked_pipe(fd: i32) -> anyhow::Result<std::fs::File> {
    use std::os::fd::FromRawFd;
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    if fd < 0 {
        bail!("private token pipe descriptor is invalid");
    }
    // SAFETY: fcntl validates the untrusted descriptor. A successful duplicate
    // is owned exclusively here and closed by File on every subsequent path.
    let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
    if duplicate < 0 {
        return Err(std::io::Error::last_os_error()).context("private pipe duplicate failed");
    }
    // SAFETY: duplicate is a fresh, valid, exclusively owned descriptor.
    let file = unsafe { std::fs::File::from_raw_fd(duplicate) };
    let metadata = file
        .metadata()
        .context("private pipe metadata unavailable")?;
    // SAFETY: geteuid has no preconditions and returns this process's OS user.
    let owner = unsafe { libc::geteuid() };
    if !metadata.file_type().is_fifo() || metadata.uid() != owner || metadata.mode() & 0o077 != 0 {
        bail!("token input must be an owner-only private pipe");
    }
    // SAFETY: the duplicate remains open and owned by file for both calls.
    let flags = unsafe { libc::fcntl(duplicate, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(duplicate, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(std::io::Error::last_os_error())
            .context("private pipe nonblocking setup failed");
    }
    Ok(file)
}

#[cfg(unix)]
async fn read_pipe(file: std::fs::File) -> anyhow::Result<SecretString> {
    use secrecy::zeroize::Zeroizing;
    use std::io::Read;
    let source = tokio::io::unix::AsyncFd::new(file).context("private pipe registration failed")?;
    let value = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        let mut bytes = Zeroizing::new(Vec::new());
        loop {
            let mut ready = source.readable().await?;
            let mut chunk = Zeroizing::new([0u8; 1024]);
            match ready.try_io(|source| {
                let mut file = source.get_ref();
                file.read(&mut chunk[..])
            }) {
                Ok(Ok(0)) => break,
                Ok(Ok(count)) => {
                    if bytes.len() + count > 8192 {
                        bail!("private token exceeds input limit");
                    }
                    bytes.extend_from_slice(&chunk[..count]);
                }
                Ok(Err(error)) => return Err(error).context("private token pipe read failed"),
                Err(_would_block) => continue,
            }
        }
        let value = std::str::from_utf8(&bytes).context("private token must be UTF-8")?;
        if value.is_empty() || value.chars().any(|c| c.is_control() || c.is_whitespace()) {
            bail!("private token format refused");
        }
        Ok(SecretString::from(value.to_owned()))
    })
    .await
    .context("private token pipe timed out")??;
    Ok(value)
}

#[cfg(unix)]
async fn import_from_pipe(
    runtime: &RebornRuntime,
    extension_id: ExtensionId,
    pipe: Option<std::fs::File>,
    expected: Option<uuid::Uuid>,
) -> anyhow::Result<SetupReceipt> {
    let surface = runtime.extension_lifecycle_surface_context();
    let scope = AuthProductScope::credential_owner(
        &ResourceScope {
            tenant_id: surface.tenant_id,
            user_id: surface.user_id,
            agent_id: surface.agent_id,
            project_id: surface.project_id,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        },
        AuthSurface::Cli,
    );
    let store = runtime.extension_management().installation_store_handle();
    let definition = store
        .get_registered_package_definition(&extension_id)
        .await?
        .ok_or_else(|| anyhow!("MCP endpoint has not been explicitly enrolled"))?;
    let manifest = store
        .get_manifest(&extension_id)
        .await?
        .ok_or_else(|| anyhow!("enrolled MCP extension must be installed before setup"))?;
    if definition.manifest_hash() != manifest.manifest_hash()
        || !store.list_installations().await?.iter().any(|row| {
            row.extension_id() == &extension_id && row.owner().visible_to(&scope.resource.user_id)
        })
    {
        bail!("enrolled MCP installation does not belong to this setup scope");
    }
    let mcp = manifest
        .resolved()
        .mcp
        .as_ref()
        .ok_or_else(|| anyhow!("private MCP setup requires an enrolled HTTP MCP package"))?;
    if mcp.registration_auth
        != ironclaw_extension_contracts::hosted_mcp::HostedMcpAuthSelection::Bearer
    {
        bail!("private MCP setup requires explicit bearer enrollment");
    }
    let url = &mcp.server;
    let target = McpServerEndpoint::literal_loopback_target(url).ok_or_else(|| {
        anyhow!("private MCP setup requires the exact enrolled loopback endpoint")
    })?;
    let mut credentials = manifest
        .resolved()
        .tools
        .iter()
        .flat_map(|tool| &tool.runtime_credentials);
    let requirement = credentials
        .next()
        .ok_or_else(|| anyhow!("enrolled MCP has no bearer account"))?;
    if credentials.any(|other| other != requirement) || requirement.audience != target {
        bail!("enrolled MCP credential audience or handle is ambiguous");
    }
    let RuntimeCredentialRequirementSource::ProductAuthAccount {
        provider,
        setup: RuntimeCredentialAccountSetup::ManualToken,
    } = &requirement.source
    else {
        bail!("private MCP setup requires a native manual-token credential account");
    };
    let provider = AuthProviderId::new(provider.as_str())?;
    let auth = runtime.product_auth_services();
    let selector = auth.runtime_credential_account_selection_service();
    let lookup = CredentialAccountSelectionRequest::new(scope.clone(), provider.clone())
        .for_extension(extension_id.clone());
    let existing = match selector
        .select_configured_account_for_binding(lookup, scope.clone())
        .await
    {
        Ok(account) => Some(account),
        Err(AuthProductError::CredentialMissing) => None,
        Err(error) => return Err(error).context("private MCP account selection failed"),
    };
    // A prior handoff may have stored the account before the caller received
    // its receipt. Recover only the scoped public reference, never its bearer.
    let Some(pipe) = pipe else {
        return Ok(SetupReceipt::Inspected(AccountInspection {
            kind: "mcp_account",
            extension_id,
            account_id: existing.map(|account| account.id),
            handle: requirement.handle.clone(),
            endpoint: url.clone(),
        }));
    };
    if expected.is_some_and(|id| existing.as_ref().map(|account| account.id.as_uuid()) != Some(id))
    {
        bail!("selected MCP account changed; private setup refused");
    }
    if existing.is_some() && expected.is_none() {
        bail!("updating an MCP account requires its public expected-account id");
    }
    let mut setup = RebornManualTokenSetupRequest::new(
        scope.clone(),
        provider.clone(),
        CredentialAccountLabel::new("Local MCP bearer")?,
        AuthContinuationRef::SetupOnly,
        chrono::Utc::now() + chrono::Duration::minutes(5),
    );
    if let Some(account) = &existing {
        setup = setup.with_update_binding(CredentialAccountUpdateBinding::from_projection(
            &account.projection(),
        ));
    }
    let challenge = auth
        .request_manual_token_setup(setup)
        .await
        .map_err(NativeAuthSetupError)?;
    let token = match read_pipe(pipe).await {
        Ok(token) => token,
        Err(error) => {
            auth.abandon_manual_token(&scope, challenge.interaction_id)
                .await
                .map_err(NativeAuthSetupError)
                .context("private MCP setup abandonment failed")?;
            return Err(error);
        }
    };
    let submitted = auth
        .submit_manual_token(RebornManualTokenSubmitRequest::new(
            scope.clone(),
            challenge.interaction_id,
            token,
        ))
        .await
        .map_err(NativeAuthSetupError)?;
    let account = selector
        .select_unique_configured_runtime_account(RuntimeCredentialAccountSelectionRequest::new(
            CredentialAccountSelectionRequest::new(scope.clone(), provider)
                .for_extension(extension_id.clone()),
            scope,
            RuntimeCredentialAccountSetup::ManualToken,
            vec![],
        ))
        .await?;
    if account.id != submitted.account_id {
        bail!("private MCP account readback disagrees with setup receipt");
    }
    let activation = execute_reborn_extension_lifecycle_command(
        runtime,
        RebornExtensionLifecycleCommand::Install {
            id: extension_id.as_str().to_string(),
        },
    )
    .await?;
    if activation.phase != ironclaw_extension_contracts::state::InstallationState::Active {
        bail!("MCP account stored but authenticated activation remains unavailable");
    }
    Ok(SetupReceipt::Imported(ImportReceipt {
        extension_id,
        account_id: account.id,
        handle: requirement.handle.clone(),
        endpoint: url.clone(),
        active: true,
    }))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::fd::AsRawFd;
    use std::os::unix::{ffi::OsStrExt, fs::OpenOptionsExt};

    fn private_pipe(bytes: &[u8]) -> std::fs::File {
        let directory = tempfile::tempdir().expect("private pipe directory");
        let fifo_path = directory.path().join("input.fifo");
        let fifo_name =
            std::ffi::CString::new(fifo_path.as_os_str().as_bytes()).expect("FIFO path");
        // SAFETY: NUL-terminated private test path and owner-only mode.
        assert_eq!(unsafe { libc::mkfifo(fifo_name.as_ptr(), 0o600) }, 0);
        let reader = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(&fifo_path)
            .expect("private reader");
        let mut writer = std::fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(&fifo_path)
            .expect("private writer");
        std::fs::remove_file(&fifo_path).expect("unlink before private input");
        writer.write_all(bytes).expect("test pipe write");
        drop(writer);
        checked_pipe(reader.as_raw_fd()).expect("private pipe accepted")
    }

    #[tokio::test]
    async fn pipe_import_accepts_one_bounded_value_and_refuses_files_and_controls() {
        use secrecy::ExposeSecret;
        let value = read_pipe(private_pipe(b"synthetic-pipe-token"))
            .await
            .expect("private input");
        assert_eq!(value.expose_secret(), "synthetic-pipe-token");
        let file = tempfile::tempfile().expect("test file");
        assert!(
            checked_pipe(file.as_raw_fd()).is_err(),
            "regular files are never token ingress"
        );
        for input in [
            b"".as_slice(),
            b"synthetic\nvalue".as_slice(),
            b"synthetic value".as_slice(),
            b"synthetic\x7fvalue".as_slice(),
            b"\xff".as_slice(),
        ] {
            let error = read_pipe(private_pipe(input))
                .await
                .expect_err("invalid secret input");
            assert!(
                !error.to_string().contains("synthetic"),
                "errors must not reflect input"
            );
        }
    }
}
