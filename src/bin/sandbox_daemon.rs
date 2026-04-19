//! Per-project sandbox tool-execution daemon.
//!
//! Runs inside the project's sandbox container. Reads NDJSON requests from
//! stdin, executes the requested tool against `/project/`, and writes one
//! NDJSON response per line to stdout.
//!
//! # Tool surface
//!
//! Only the five sandbox-eligible tools are available:
//!
//! - `file_read` (also accepts `read_file`)
//! - `file_write` (also accepts `write_file`)
//! - `list_dir`
//! - `apply_patch`
//! - `shell`
//!
//! Each filesystem tool is constructed with `base_dir = /project/` (override
//! via `IRONCLAW_SANDBOX_BASE_DIR` for local testing). The shell tool runs
//! commands with `working_dir = /project/`.
//!
//! # Wire protocol
//!
//! Each request is one JSON object on a single line:
//!
//! ```json
//! {"id":"<uuid>","method":"execute_tool","params":{"name":"file_read","input":{"path":"/project/foo.txt"},"context":{}}}
//! ```
//!
//! Each response is one JSON object on a single line:
//!
//! ```json
//! {"id":"<uuid>","result":{"output":{...},"metadata":{"duration_ms":12}}}
//! ```
//!
//! Errors:
//!
//! ```json
//! {"id":"<uuid>","error":{"code":"tool_error","message":"...","details":{}}}
//! ```
//!
//! Other methods (no params required):
//!
//! - `health` → `{"result":{"status":"ok","tools":[...]}}`
//! - `shutdown` → drains in-flight calls then exits cleanly
//!
//! # Concurrency
//!
//! v1 serializes: one request at a time. The host-side
//! `ContainerizedFilesystemBackend` queues calls, so this is fine. A future
//! optimization can multiplex via the request `id` field.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use ironclaw::bridge::sandbox::protocol::{Request, Response, RpcError, SUPPORTED_TOOLS};
use ironclaw::context::JobContext;
use ironclaw::tools::builtin::{
    ApplyPatchTool, ListDirTool, ReadFileTool, ShellTool, WriteFileTool,
};
use ironclaw::tools::{Tool, ToolError, ToolOutput};

/// Default project mount path inside the container.
const DEFAULT_BASE_DIR: &str = "/project";

struct Daemon {
    base_dir: PathBuf,
    read_file: Arc<ReadFileTool>,
    write_file: Arc<WriteFileTool>,
    list_dir: Arc<ListDirTool>,
    apply_patch: Arc<ApplyPatchTool>,
    shell: Arc<ShellTool>,
}

impl Daemon {
    fn new(base_dir: PathBuf) -> Self {
        Self {
            read_file: Arc::new(ReadFileTool::new().with_base_dir(base_dir.clone())),
            write_file: Arc::new(WriteFileTool::new().with_base_dir(base_dir.clone())),
            list_dir: Arc::new(ListDirTool::new().with_base_dir(base_dir.clone())),
            apply_patch: Arc::new(ApplyPatchTool::new().with_base_dir(base_dir.clone())),
            shell: Arc::new(ShellTool::new().with_working_dir(base_dir.clone())),
            base_dir,
        }
    }

    fn job_ctx(&self) -> JobContext {
        JobContext::with_user(
            "sandbox",
            "sandbox_daemon",
            "per-project sandbox tool dispatch",
        )
    }

    async fn execute_tool(&self, name: &str, input: Value) -> Result<ToolOutput, ToolError> {
        let ctx = self.job_ctx();
        match name {
            // Accept both engine v2 and v1 tool names so the daemon doesn't
            // care which name the caller uses. Plan calls these "file_read"
            // / "file_write" but the host's tools use "read_file" / "write_file"
            // — both map to the same struct.
            "file_read" | "read_file" => self.read_file.execute(input, &ctx).await,
            "file_write" | "write_file" => self.write_file.execute(input, &ctx).await,
            "list_dir" => self.list_dir.execute(input, &ctx).await,
            "apply_patch" => self.apply_patch.execute(input, &ctx).await,
            "shell" => self.shell.execute(input, &ctx).await,
            _ => Err(ToolError::ExecutionFailed(format!(
                "sandbox_daemon: unknown tool '{name}' (supported: {})",
                SUPPORTED_TOOLS.join(", ")
            ))),
        }
    }

    async fn handle(&self, req: Request) -> Response {
        let id = Some(req.id.clone());
        match req.method.as_str() {
            "health" => Response {
                id,
                result: Some(serde_json::json!({
                    "status": "ok",
                    "base_dir": self.base_dir.display().to_string(),
                    "tools": SUPPORTED_TOOLS,
                })),
                error: None,
            },
            "shutdown" => Response {
                id,
                result: Some(serde_json::json!({"status": "shutting_down"})),
                error: None,
            },
            "execute_tool" => {
                let name = match req.params.get("name").and_then(|v| v.as_str()) {
                    Some(n) => n.to_string(),
                    None => {
                        return Response {
                            id,
                            result: None,
                            error: Some(RpcError::new(
                                "invalid_params",
                                "execute_tool requires params.name (string)",
                            )),
                        };
                    }
                };
                let input = req
                    .params
                    .get("input")
                    .cloned()
                    .unwrap_or(Value::Object(Default::default()));
                let started = Instant::now();
                match self.execute_tool(&name, input).await {
                    Ok(out) => {
                        let duration_ms = started.elapsed().as_millis() as u64;
                        Response {
                            id,
                            result: Some(serde_json::json!({
                                "output": out.result,
                                "metadata": { "duration_ms": duration_ms },
                            })),
                            error: None,
                        }
                    }
                    Err(e) => Response {
                        id,
                        result: None,
                        error: Some(map_tool_error(e)),
                    },
                }
            }
            other => Response {
                id,
                result: None,
                error: Some(RpcError::new(
                    "unknown_method",
                    format!("unknown method '{other}'"),
                )),
            },
        }
    }
}

fn map_tool_error(err: ToolError) -> RpcError {
    let (code, message) = match &err {
        ToolError::InvalidParameters(m) => ("invalid_params", m.clone()),
        ToolError::RateLimited(_) => ("rate_limited", err.to_string()),
        ToolError::ExternalService(m) => ("tool_error", m.clone()),
        ToolError::Sandbox(m) => ("sandbox_error", m.clone()),
        _ => ("tool_error", err.to_string()),
    };
    RpcError::new(code, message)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    // Initialize tracing only on stderr — stdout is the protocol channel.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("IRONCLAW_SANDBOX_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let base_dir = std::env::var("IRONCLAW_SANDBOX_BASE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_BASE_DIR));
    tracing::info!(base_dir = %base_dir.display(), "sandbox_daemon starting");

    let daemon = Daemon::new(base_dir);

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin).lines();
    let mut stdout = tokio::io::stdout();

    while let Some(line) = reader.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let req: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = Response {
                    id: None,
                    result: None,
                    error: Some(RpcError::new(
                        "parse_error",
                        format!("invalid JSON request: {e}"),
                    )),
                };
                write_response(&mut stdout, &resp).await?;
                continue;
            }
        };

        let is_shutdown = req.method == "shutdown";
        let resp = daemon.handle(req).await;
        write_response(&mut stdout, &resp).await?;
        if is_shutdown {
            tracing::info!("sandbox_daemon: shutdown received, exiting");
            break;
        }
    }

    Ok(())
}

async fn write_response(stdout: &mut tokio::io::Stdout, resp: &Response) -> std::io::Result<()> {
    let mut bytes = serde_json::to_vec(resp).map_err(std::io::Error::other)?;
    bytes.push(b'\n');
    stdout.write_all(&bytes).await?;
    stdout.flush().await
}
