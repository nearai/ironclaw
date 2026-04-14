//! [`ContainerizedFilesystemBackend`] — [`MountBackend`] for the per-project
//! sandbox container.
//!
//! Each backend instance owns an [`Arc<dyn SandboxTransport>`] (typically
//! [`super::docker_transport::DockerTransport`]) and serializes
//! filesystem/shell calls into JSON-RPC requests for the daemon running
//! inside the project's container.
//!
//! Path semantics are identical to [`ironclaw_engine::workspace::FilesystemBackend`]:
//! the backend receives **relative** paths (the bridge interceptor strips
//! the `/project/` prefix before calling). The daemon's tools are configured
//! with `base_dir = /project/`, so the relative path is re-anchored at
//! `/project/<rel>` inside the container.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_engine::workspace::{DirEntry, EntryKind, ShellOutput};
use ironclaw_engine::{MountBackend, MountError};
use serde_json::Value;
use uuid::Uuid;

use super::protocol::{Request, Response, RpcError};
use super::transport::SandboxTransport;

/// All daemon paths live under `/project/` inside the container. The host
/// translates relative paths into absolute container paths here so the
/// daemon's `base_dir` validation accepts them.
const CONTAINER_PROJECT_ROOT: &str = "/project";

/// [`MountBackend`] backed by a per-project sandbox container.
#[derive(Debug, Clone)]
pub struct ContainerizedFilesystemBackend {
    transport: Arc<dyn SandboxTransport>,
}

impl ContainerizedFilesystemBackend {
    pub fn new(transport: Arc<dyn SandboxTransport>) -> Self {
        Self { transport }
    }

    /// Build the absolute container path for a relative mount path.
    fn container_path(rel: &Path) -> String {
        let rel_str = rel.to_string_lossy();
        if rel_str.is_empty() {
            CONTAINER_PROJECT_ROOT.to_string()
        } else {
            format!(
                "{CONTAINER_PROJECT_ROOT}/{}",
                rel_str.trim_start_matches('/')
            )
        }
    }

    /// Run an `execute_tool` JSON-RPC call and unwrap the standard
    /// `result.output` envelope, mapping daemon-side `error` payloads into
    /// the appropriate [`MountError`] variants.
    async fn run_tool(&self, tool: &str, input: Value) -> Result<Value, MountError> {
        let request = Request::execute_tool(Uuid::new_v4().to_string(), tool, input);
        let response = self.transport.dispatch(request).await?;
        unwrap_tool_response(tool, response)
    }
}

fn unwrap_tool_response(tool: &str, response: Response) -> Result<Value, MountError> {
    if let Some(err) = response.error {
        return Err(map_rpc_error(tool, err));
    }
    let result = response.result.ok_or_else(|| MountError::Backend {
        reason: format!("daemon returned neither result nor error for {tool}"),
    })?;
    let output = result
        .get("output")
        .cloned()
        .ok_or_else(|| MountError::Backend {
            reason: format!("daemon returned result without 'output' key for {tool}"),
        })?;
    Ok(output)
}

/// Map daemon RPC errors to [`MountError`] so the bridge surfaces them
/// consistently with the [`ironclaw_engine::workspace::FilesystemBackend`] equivalents.
fn map_rpc_error(tool: &str, err: RpcError) -> MountError {
    match err.code.as_str() {
        "tool_error" if err.message.contains("not found") || err.message.contains("No such") => {
            MountError::NotFound {
                path: err.message.clone(),
            }
        }
        "tool_error" if err.message.contains("permission denied") => MountError::PermissionDenied {
            path: err.message.clone(),
        },
        "tool_error" => MountError::Tool {
            reason: format!("{tool}: {}", err.message),
        },
        "invalid_params" => MountError::InvalidPath {
            path: String::new(),
            reason: err.message,
        },
        "rate_limited" => MountError::Tool {
            reason: format!("rate limited: {}", err.message),
        },
        "sandbox_error" | "backend" => MountError::Backend {
            reason: format!("{tool}: {}", err.message),
        },
        "parse_error" | "unknown_method" | "unknown_tool" => MountError::Backend {
            reason: format!("{tool}: protocol error: {}", err.message),
        },
        _ => MountError::Backend {
            reason: format!("{tool}: {} ({})", err.message, err.code),
        },
    }
}

#[async_trait]
impl MountBackend for ContainerizedFilesystemBackend {
    async fn read(&self, rel_path: &Path) -> Result<Vec<u8>, MountError> {
        let path = Self::container_path(rel_path);
        let output = self
            .run_tool("file_read", serde_json::json!({"path": path}))
            .await?;
        let content = output
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| MountError::Backend {
                reason: "file_read response missing content".into(),
            })?;
        Ok(content.as_bytes().to_vec())
    }

    async fn write(&self, rel_path: &Path, content: &[u8]) -> Result<(), MountError> {
        let path = Self::container_path(rel_path);
        let body = std::str::from_utf8(content).map_err(|_| MountError::InvalidPath {
            path: path.clone(),
            reason: "binary content is not supported in the sandbox wire protocol".into(),
        })?;
        self.run_tool(
            "file_write",
            serde_json::json!({"path": path, "content": body}),
        )
        .await?;
        Ok(())
    }

    async fn list(&self, rel_path: &Path, depth: usize) -> Result<Vec<DirEntry>, MountError> {
        let path = Self::container_path(rel_path);
        let recursive = depth > 0;
        let output = self
            .run_tool(
                "list_dir",
                serde_json::json!({
                    "path": path,
                    "recursive": recursive,
                    "max_depth": depth.max(1),
                }),
            )
            .await?;
        let entries = output
            .get("entries")
            .and_then(|v| v.as_array())
            .ok_or_else(|| MountError::Backend {
                reason: "list_dir response missing entries".into(),
            })?;
        // Daemon returns formatted strings like "foo.txt (1.2K)" or "sub/".
        // We parse them back into DirEntry minimally — the bridge interceptor
        // re-formats this on the way to the LLM, so we just need correct
        // path + kind. Sizes round-trip best-effort.
        let mut out = Vec::with_capacity(entries.len());
        for entry in entries {
            let raw = match entry.as_str() {
                Some(s) => s,
                None => continue,
            };
            let (path_part, kind) = if let Some(prefix) = raw.strip_suffix('/') {
                (prefix.to_string(), EntryKind::Directory)
            } else if let Some((name, _)) = raw.rsplit_once(" (") {
                (name.to_string(), EntryKind::File)
            } else {
                (raw.to_string(), EntryKind::File)
            };
            out.push(DirEntry {
                path: PathBuf::from(path_part),
                kind,
                size: None,
            });
        }
        Ok(out)
    }

    async fn patch(
        &self,
        rel_path: &Path,
        old_string: &str,
        new_string: &str,
        replace_all: bool,
    ) -> Result<(), MountError> {
        let path = Self::container_path(rel_path);
        self.run_tool(
            "apply_patch",
            serde_json::json!({
                "path": path,
                "old_string": old_string,
                "new_string": new_string,
                "replace_all": replace_all,
            }),
        )
        .await?;
        Ok(())
    }

    async fn shell(
        &self,
        command: &str,
        env: HashMap<String, String>,
        cwd: Option<&Path>,
    ) -> Result<ShellOutput, MountError> {
        let workdir = match cwd {
            Some(p) => Self::container_path(p),
            None => CONTAINER_PROJECT_ROOT.to_string(),
        };
        let output = self
            .run_tool(
                "shell",
                serde_json::json!({
                    "command": command,
                    "workdir": workdir,
                    "env": env,
                }),
            )
            .await?;
        // The daemon forwards the host `ShellTool`'s raw result. That shape
        // uses `output` for merged stdout+stderr (and has no separate `stderr`
        // field). Our `ShellOutput` exposes stdout/stderr/exit_code, so we
        // fold the merged stream into `stdout` and leave `stderr` empty when
        // no explicit field is present. This also accepts the canonical
        // `{stdout, stderr, exit_code}` shape so a future daemon change or a
        // different tool impl works without another patch.
        let stdout = output
            .get("stdout")
            .and_then(|v| v.as_str())
            .or_else(|| output.get("output").and_then(|v| v.as_str()))
            .unwrap_or("")
            .to_string();
        let stderr = output
            .get("stderr")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let exit_code = output
            .get("exit_code")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;
        Ok(ShellOutput {
            stdout,
            stderr,
            exit_code,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tokio::sync::Notify;

    /// In-process transport that records every request and returns
    /// scripted responses. Used by the tests below to verify the backend
    /// translates correctly between `MountBackend` calls and JSON-RPC.
    #[derive(Debug)]
    struct ScriptedTransport {
        captured: Mutex<Vec<Request>>,
        responses: Mutex<Vec<Response>>,
    }

    impl ScriptedTransport {
        fn new(responses: Vec<Response>) -> Arc<Self> {
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
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                Err(MountError::Backend {
                    reason: "no scripted response left".into(),
                })
            } else {
                Ok(responses.remove(0))
            }
        }
    }

    fn ok_resp(output: Value) -> Response {
        Response {
            id: Some("x".into()),
            result: Some(serde_json::json!({"output": output})),
            error: None,
        }
    }

    #[tokio::test]
    async fn read_translates_to_execute_tool_request() {
        let transport = ScriptedTransport::new(vec![ok_resp(serde_json::json!({"content": "hi"}))]);
        let backend = ContainerizedFilesystemBackend::new(transport.clone());

        let bytes = backend.read(Path::new("foo.txt")).await.unwrap();
        assert_eq!(bytes, b"hi");

        let captured = transport.captured.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].method, "execute_tool");
        assert_eq!(captured[0].params["name"], "file_read");
        assert_eq!(captured[0].params["input"]["path"], "/project/foo.txt");
    }

    #[tokio::test]
    async fn write_sends_path_and_content() {
        let transport =
            ScriptedTransport::new(vec![ok_resp(serde_json::json!({"bytes_written": 5}))]);
        let backend = ContainerizedFilesystemBackend::new(transport.clone());

        backend
            .write(Path::new("hello.txt"), b"world")
            .await
            .unwrap();

        let captured = transport.captured.lock().unwrap();
        assert_eq!(captured[0].params["name"], "file_write");
        assert_eq!(captured[0].params["input"]["path"], "/project/hello.txt");
        assert_eq!(captured[0].params["input"]["content"], "world");
    }

    #[tokio::test]
    async fn list_parses_daemon_entries() {
        let transport = ScriptedTransport::new(vec![ok_resp(serde_json::json!({
            "entries": ["sub/", "a.txt (1.2K)", "b.txt (44)"]
        }))]);
        let backend = ContainerizedFilesystemBackend::new(transport.clone());

        let entries = backend.list(Path::new(""), 0).await.unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].kind, EntryKind::Directory);
        assert_eq!(entries[0].path, Path::new("sub"));
        assert_eq!(entries[1].kind, EntryKind::File);
        assert_eq!(entries[1].path, Path::new("a.txt"));

        // Path uses /project root for empty rel
        let captured = transport.captured.lock().unwrap();
        assert_eq!(captured[0].params["input"]["path"], "/project");
    }

    /// Regression for the "silent shell loop" bug caught by the live e2e
    /// test. The host's `ShellTool` returns `{exit_code, output, ...}` with
    /// merged stdout+stderr in the `output` field — no `stdout` key. The
    /// backend must fold `output` into `ShellOutput.stdout` so the agent
    /// actually sees command output, otherwise it re-runs `git clone` in a
    /// loop because every response looks like silent success.
    #[tokio::test]
    async fn shell_accepts_merged_output_field_from_host_tool() {
        let transport = ScriptedTransport::new(vec![ok_resp(serde_json::json!({
            "exit_code": 0,
            "output": "Cloning into '/project/repo'...\n",
            "sandboxed": false,
            "success": true,
        }))]);
        let backend = ContainerizedFilesystemBackend::new(transport);
        let out = backend
            .shell("git clone ...", HashMap::new(), None)
            .await
            .unwrap();
        assert!(
            out.stdout.contains("Cloning into"),
            "merged `output` field should be exposed as stdout, got: {out:?}"
        );
        assert_eq!(out.exit_code, 0);
    }

    #[tokio::test]
    async fn shell_returns_stdout_and_exit_code() {
        let transport = ScriptedTransport::new(vec![ok_resp(serde_json::json!({
            "stdout": "hello\n",
            "stderr": "",
            "exit_code": 0,
        }))]);
        let backend = ContainerizedFilesystemBackend::new(transport.clone());

        let env = HashMap::from([("FOO".to_string(), "bar".to_string())]);
        let out = backend
            .shell("echo hello", env.clone(), Some(Path::new("sub")))
            .await
            .unwrap();
        assert_eq!(out.stdout, "hello\n");
        assert_eq!(out.exit_code, 0);

        let captured = transport.captured.lock().unwrap();
        assert_eq!(captured[0].params["input"]["command"], "echo hello");
        assert_eq!(captured[0].params["input"]["workdir"], "/project/sub");
        assert_eq!(captured[0].params["input"]["env"]["FOO"], "bar");
    }

    #[tokio::test]
    async fn tool_error_maps_to_mount_error_variants() {
        // not found
        let transport = ScriptedTransport::new(vec![Response {
            id: Some("x".into()),
            result: None,
            error: Some(RpcError {
                code: "tool_error".into(),
                message: "file not found: /project/missing".into(),
                details: Value::Null,
            }),
        }]);
        let backend = ContainerizedFilesystemBackend::new(transport);
        let err = backend.read(Path::new("missing")).await.unwrap_err();
        assert!(matches!(err, MountError::NotFound { .. }));

        // sandbox error
        let transport = ScriptedTransport::new(vec![Response {
            id: Some("x".into()),
            result: None,
            error: Some(RpcError {
                code: "sandbox_error".into(),
                message: "daemon crashed".into(),
                details: Value::Null,
            }),
        }]);
        let backend = ContainerizedFilesystemBackend::new(transport);
        let err = backend.read(Path::new("x")).await.unwrap_err();
        assert!(matches!(err, MountError::Backend { .. }));
    }

    /// **Test through the caller, not just the helper.** This drives a
    /// "shell write then file_read" sequence — the same path the integration
    /// tests will exercise — to verify both calls reach the transport in
    /// order, and that the relative→absolute path translation matches the
    /// daemon's expectations.
    #[tokio::test]
    async fn end_to_end_request_sequence() {
        let transport = ScriptedTransport::new(vec![
            ok_resp(serde_json::json!({"stdout": "", "stderr": "", "exit_code": 0})),
            ok_resp(serde_json::json!({"content": "hello"})),
        ]);
        let backend = ContainerizedFilesystemBackend::new(transport.clone());

        backend
            .shell("echo hello > foo.txt", HashMap::new(), None)
            .await
            .unwrap();
        let bytes = backend.read(Path::new("foo.txt")).await.unwrap();
        assert_eq!(bytes, b"hello");

        let captured = transport.captured.lock().unwrap();
        assert_eq!(captured.len(), 2);
        assert_eq!(captured[0].params["name"], "shell");
        assert_eq!(captured[1].params["name"], "file_read");
        // Workdir defaults to /project when no cwd is passed.
        assert_eq!(captured[0].params["input"]["workdir"], "/project");
    }

    /// Notify is dragged in just so the unused-import linter is happy if a
    /// future test wants to coordinate via async wakeups. No-op today.
    #[allow(dead_code)]
    fn _notify_link() {
        let _ = Notify::new();
    }
}
