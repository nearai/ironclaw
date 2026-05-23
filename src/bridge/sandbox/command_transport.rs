//! Command transport adapter for Reborn tenant-sandbox process execution.
//!
//! This is the V1 sandbox-daemon bridge behind Reborn's placement-neutral
//! `SandboxCommandTransport` trait. First-party tools still talk only to
//! `InvocationServices.process`; this adapter maps that abstract process effect
//! to the existing Docker/NDJSON `execute_tool("shell", ...)` protocol.

use std::path::{Component, Path};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use ironclaw_host_runtime::{
    CommandExecutionOutput, CommandExecutionRequest, RuntimeProcessError, SandboxCommandTransport,
};
use serde_json::Value;
use uuid::Uuid;

use super::protocol::{Request, Response, RpcError};
use super::transport::SandboxTransport;

const CONTAINER_PROJECT_ROOT: &str = "/project";
const DEFAULT_TIMEOUT_SECS: u64 = 120;
const MAX_MERGED_OUTPUT_BYTES: usize = 64 * 1024;

/// Reborn process-command transport backed by the V1 sandbox daemon.
#[derive(Debug, Clone)]
pub(crate) struct DockerSandboxCommandTransport {
    transport: Arc<dyn SandboxTransport>,
}

impl DockerSandboxCommandTransport {
    pub(crate) fn new(transport: Arc<dyn SandboxTransport>) -> Self {
        Self { transport }
    }

    async fn run_tool(&self, input: Value) -> Result<Value, RuntimeProcessError> {
        let request = Request::execute_tool(Uuid::new_v4().to_string(), "shell", input);
        let response = self.transport.dispatch(request).await.map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!("sandbox transport failed: {error}"))
        })?;
        unwrap_shell_response(response)
    }
}

#[async_trait]
impl SandboxCommandTransport for DockerSandboxCommandTransport {
    async fn run_command(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        if request.command.contains('\0') {
            return Err(RuntimeProcessError::ExecutionFailed(
                "sandbox shell command must not contain null bytes".to_string(),
            ));
        }
        if !request.extra_env.is_empty() {
            return Err(RuntimeProcessError::ExecutionFailed(
                "sandbox shell environment overrides are not supported yet".to_string(),
            ));
        }
        if request
            .mounts
            .as_ref()
            .is_some_and(|mounts| !mounts.mounts.is_empty())
        {
            return Err(RuntimeProcessError::ExecutionFailed(
                "sandbox shell scoped mounts are not supported yet".to_string(),
            ));
        }

        let workdir = container_workdir(request.workdir.as_deref())?;
        let timeout_secs = request.timeout_secs;
        let timeout = Duration::from_secs(timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS));
        let start = Instant::now();
        let output = match tokio::time::timeout(
            timeout,
            self.run_tool(serde_json::json!({
                "command": request.command,
                "workdir": workdir,
                "timeout": timeout_secs,
            })),
        )
        .await
        {
            Ok(result) => result.map_err(|error| map_shell_timeout(error, timeout_secs))?,
            Err(_) => {
                self.transport.reset().await;
                return Err(RuntimeProcessError::Timeout(timeout));
            }
        };
        let (merged_output, exit_code) = parse_shell_output(output)?;
        Ok(CommandExecutionOutput {
            output: merged_output,
            exit_code,
            sandboxed: true,
            duration: start.elapsed(),
        })
    }
}

fn unwrap_shell_response(response: Response) -> Result<Value, RuntimeProcessError> {
    if let Some(error) = response.error {
        return Err(map_rpc_error(error));
    }
    let result = response.result.ok_or_else(|| {
        RuntimeProcessError::ExecutionFailed(
            "sandbox daemon returned neither result nor error for shell".to_string(),
        )
    })?;
    result.get("output").cloned().ok_or_else(|| {
        RuntimeProcessError::ExecutionFailed(
            "sandbox daemon shell result missing output".to_string(),
        )
    })
}

fn map_rpc_error(error: RpcError) -> RuntimeProcessError {
    RuntimeProcessError::ExecutionFailed(format!(
        "sandbox shell failed: {} ({})",
        error.message, error.code
    ))
}

fn map_shell_timeout(error: RuntimeProcessError, timeout_secs: Option<u64>) -> RuntimeProcessError {
    match &error {
        RuntimeProcessError::ExecutionFailed(message)
            if message.to_ascii_lowercase().contains("timed out") =>
        {
            RuntimeProcessError::Timeout(Duration::from_secs(
                timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS),
            ))
        }
        _ => error,
    }
}

fn parse_shell_output(output: Value) -> Result<(String, i64), RuntimeProcessError> {
    let stdout = output
        .get("stdout")
        .and_then(Value::as_str)
        .or_else(|| output.get("output").and_then(Value::as_str));
    let stderr = output.get("stderr").and_then(Value::as_str).unwrap_or("");
    if stdout.is_none() && stderr.is_empty() {
        return Err(RuntimeProcessError::ExecutionFailed(
            "sandbox daemon shell output missing stdout/output".to_string(),
        ));
    }
    let stdout = stdout.unwrap_or("");
    let exit_code = output
        .get("exit_code")
        .and_then(Value::as_i64)
        .ok_or_else(|| {
            RuntimeProcessError::ExecutionFailed(
                "sandbox daemon shell output missing exit_code".to_string(),
            )
        })?;

    let merged = if stdout.is_empty() {
        stderr.to_string()
    } else if stderr.is_empty() {
        stdout.to_string()
    } else {
        let separator = if stdout.ends_with('\n') { "" } else { "\n" };
        format!("{stdout}{separator}{stderr}")
    };
    Ok((truncate_merged_output(&merged), exit_code))
}

fn truncate_merged_output(output: &str) -> String {
    if output.len() <= MAX_MERGED_OUTPUT_BYTES {
        return output.to_string();
    }
    let mut end = MAX_MERGED_OUTPUT_BYTES;
    while !output.is_char_boundary(end) {
        end -= 1;
    }
    let truncated = &output[..end]; // safety: end is adjusted to a UTF-8 char boundary before slicing.
    format!("{truncated}... [truncated]")
}

fn container_workdir(workdir: Option<&str>) -> Result<String, RuntimeProcessError> {
    let Some(workdir) = workdir.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(CONTAINER_PROJECT_ROOT.to_string());
    };
    if workdir.contains('\0') {
        return Err(RuntimeProcessError::ExecutionFailed(
            "sandbox shell workdir must not contain null bytes".to_string(),
        ));
    }
    if workdir == CONTAINER_PROJECT_ROOT {
        return Ok(CONTAINER_PROJECT_ROOT.to_string());
    }
    let relative = if let Some(relative) = workdir.strip_prefix("/project/") {
        relative
    } else if workdir.starts_with('/') {
        return Err(RuntimeProcessError::ExecutionFailed(format!(
            "sandbox shell workdir must be under {CONTAINER_PROJECT_ROOT}: {workdir}"
        )));
    } else {
        workdir
    };
    validate_relative_workdir(relative)?;
    Ok(format!(
        "{CONTAINER_PROJECT_ROOT}/{}",
        relative.trim_start_matches('/')
    ))
}

fn validate_relative_workdir(path: &str) -> Result<(), RuntimeProcessError> {
    for component in Path::new(path).components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox shell workdir contains unsupported path component: {path}"
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_engine::MountError;
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug)]
    struct ScriptedTransport {
        captured: Mutex<Vec<Request>>,
        responses: Mutex<Vec<Result<Response, MountError>>>,
    }

    impl ScriptedTransport {
        fn new(responses: Vec<Result<Response, MountError>>) -> Arc<Self> {
            Arc::new(Self {
                captured: Mutex::new(Vec::new()),
                responses: Mutex::new(responses),
            })
        }
    }

    #[async_trait]
    impl SandboxTransport for ScriptedTransport {
        async fn dispatch(&self, request: Request) -> Result<Response, MountError> {
            self.captured.lock().unwrap().push(request);
            self.responses.lock().unwrap().remove(0)
        }
    }

    #[derive(Debug, Default)]
    struct SlowTransport {
        reset_count: AtomicUsize,
    }

    #[async_trait]
    impl SandboxTransport for SlowTransport {
        async fn dispatch(&self, _request: Request) -> Result<Response, MountError> {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok(ok_response(serde_json::json!({
                "stdout": "too late",
                "exit_code": 0,
            })))
        }

        async fn reset(&self) {
            self.reset_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn ok_response(output: Value) -> Response {
        Response {
            id: Some("x".to_string()),
            result: Some(serde_json::json!({"output": output})),
            error: None,
        }
    }

    fn request(command: &str) -> CommandExecutionRequest {
        CommandExecutionRequest {
            scope: ironclaw_host_api::ResourceScope::system(),
            mounts: None,
            command: command.to_string(),
            workdir: None,
            timeout_secs: Some(7),
            extra_env: Default::default(),
        }
    }

    fn non_empty_mounts() -> MountView {
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").unwrap(),
            VirtualPath::new("/projects/workspace").unwrap(),
            MountPermissions::read_only(),
        )])
        .unwrap()
    }

    #[tokio::test]
    async fn dispatches_shell_execute_tool_request() {
        let transport = ScriptedTransport::new(vec![Ok(ok_response(serde_json::json!({
            "stdout": "hi\n",
            "stderr": "",
            "exit_code": 0,
        })))]);
        let adapter = DockerSandboxCommandTransport::new(transport.clone());

        let output = adapter.run_command(request("printf hi")).await.unwrap();
        assert_eq!(output.output, "hi\n");
        assert_eq!(output.exit_code, 0);
        assert!(output.sandboxed);

        let captured = transport.captured.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].method, "execute_tool");
        assert_eq!(captured[0].params["name"], "shell");
        assert_eq!(captured[0].params["input"]["command"], "printf hi");
        assert_eq!(captured[0].params["input"]["workdir"], "/project");
        assert_eq!(captured[0].params["input"]["timeout"], 7);
    }

    #[tokio::test]
    async fn rejects_null_byte_command() {
        let transport = ScriptedTransport::new(vec![]);
        let adapter = DockerSandboxCommandTransport::new(transport);

        let error = adapter
            .run_command(request("printf ok\0ignored"))
            .await
            .unwrap_err();

        assert!(format!("{error}").contains("null bytes"));
    }

    #[tokio::test]
    async fn accepts_merged_daemon_output_field() {
        let transport = ScriptedTransport::new(vec![Ok(ok_response(serde_json::json!({
            "output": "compiler says no",
            "success": false,
            "exit_code": 2,
        })))]);
        let adapter = DockerSandboxCommandTransport::new(transport);

        let output = adapter.run_command(request("cargo test")).await.unwrap();
        assert_eq!(output.output, "compiler says no");
        assert_eq!(output.exit_code, 2);
    }

    #[test]
    fn workdir_must_stay_inside_project_root() {
        assert_eq!(container_workdir(None).unwrap(), "/project");
        assert_eq!(
            container_workdir(Some("sub/dir")).unwrap(),
            "/project/sub/dir"
        );
        assert_eq!(
            container_workdir(Some("/project/sub")).unwrap(),
            "/project/sub"
        );
        assert!(container_workdir(Some("/tmp")).is_err());
        assert!(container_workdir(Some("../escape")).is_err());
        assert!(container_workdir(Some("/project/../escape")).is_err());
        assert!(container_workdir(Some("/project/\0/../etc")).is_err());
    }

    #[test]
    fn container_workdir_handles_empty_and_whitespace_only_input() {
        assert_eq!(container_workdir(Some("")).unwrap(), "/project");
        assert_eq!(container_workdir(Some("   ")).unwrap(), "/project");
        assert_eq!(container_workdir(Some("/project/")).unwrap(), "/project/");
    }

    #[tokio::test]
    async fn rejects_env_until_daemon_contract_supports_it() {
        let transport = ScriptedTransport::new(vec![]);
        let adapter = DockerSandboxCommandTransport::new(transport);
        let mut command = request("env");
        command.extra_env.insert("A".to_string(), "B".to_string());

        let error = adapter.run_command(command).await.unwrap_err();
        assert!(format!("{error}").contains("environment overrides"));
    }

    #[tokio::test]
    async fn rejects_scoped_mounts_until_daemon_contract_supports_it() {
        let transport = ScriptedTransport::new(vec![]);
        let adapter = DockerSandboxCommandTransport::new(transport);
        let mut command = request("pwd");
        command.mounts = Some(non_empty_mounts());

        let error = adapter.run_command(command).await.unwrap_err();
        assert!(format!("{error}").contains("scoped mounts"));
    }

    #[tokio::test]
    async fn enforces_timeout_at_transport_boundary_and_resets_session() {
        let transport = Arc::new(SlowTransport::default());
        let adapter = DockerSandboxCommandTransport::new(transport.clone());
        let mut command = request("sleep 60");
        command.timeout_secs = Some(2);

        let error = adapter.run_command(command).await.unwrap_err();

        assert_eq!(error, RuntimeProcessError::Timeout(Duration::from_secs(2)));
        assert_eq!(transport.reset_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn maps_daemon_timeout_to_process_timeout() {
        let transport = ScriptedTransport::new(vec![Ok(Response {
            id: Some("x".to_string()),
            result: None,
            error: Some(RpcError::new("tool_error", "timed out after 7s")),
        })]);
        let adapter = DockerSandboxCommandTransport::new(transport);

        let error = adapter.run_command(request("sleep 100")).await.unwrap_err();
        assert_eq!(error, RuntimeProcessError::Timeout(Duration::from_secs(7)));
    }

    #[tokio::test]
    async fn transport_dispatch_error_maps_to_runtime_process_error() {
        let transport = ScriptedTransport::new(vec![Err(MountError::Backend {
            reason: "docker exec failed".to_string(),
        })]);
        let adapter = DockerSandboxCommandTransport::new(transport);

        let error = adapter.run_command(request("pwd")).await.unwrap_err();

        assert!(format!("{error}").contains("sandbox transport failed"));
        assert!(format!("{error}").contains("docker exec failed"));
    }

    #[tokio::test]
    async fn daemon_malformed_responses_are_rejected() {
        let cases = [
            Response {
                id: Some("x".to_string()),
                result: None,
                error: None,
            },
            Response {
                id: Some("x".to_string()),
                result: Some(serde_json::json!({})),
                error: None,
            },
            ok_response(serde_json::json!({
                "stdout": "missing exit code",
            })),
            ok_response(serde_json::json!({
                "exit_code": 0,
            })),
            ok_response(serde_json::json!({
                "stdout": null,
                "output": null,
                "exit_code": 0,
            })),
        ];

        for response in cases {
            let transport = ScriptedTransport::new(vec![Ok(response)]);
            let adapter = DockerSandboxCommandTransport::new(transport);

            let error = adapter.run_command(request("pwd")).await.unwrap_err();

            assert!(matches!(error, RuntimeProcessError::ExecutionFailed(_)));
        }
    }

    #[tokio::test]
    async fn merges_stdout_and_stderr_without_double_spacing() {
        let transport = ScriptedTransport::new(vec![Ok(ok_response(serde_json::json!({
            "stdout": "out\n",
            "stderr": "err",
            "exit_code": 0,
        })))]);
        let adapter = DockerSandboxCommandTransport::new(transport);

        let output = adapter.run_command(request("cargo test")).await.unwrap();

        assert_eq!(output.output, "out\nerr");
    }

    #[tokio::test]
    async fn missing_stdout_and_output_fields_are_rejected() {
        let transport = ScriptedTransport::new(vec![Ok(ok_response(serde_json::json!({
            "exit_code": 0,
        })))]);
        let adapter = DockerSandboxCommandTransport::new(transport);

        let error = adapter.run_command(request("pwd")).await.unwrap_err();

        assert!(format!("{error}").contains("missing stdout/output"));
    }
}
