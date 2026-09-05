//! Loop-worker conformance: drive real worker binaries through the host
//! membrane over local stdio (no Docker).
//!
//! Two lanes share one scripted host double:
//!
//! - the canonical Rust worker (`env!("CARGO_BIN_EXE_ironclaw-loop-worker")`,
//!   same crate, so the cargo-provided env resolves without a new dev-dependency
//!   edge), bootstrapped content-`Blind`;
//! - the Pi worker (`bun run docker/sandbox/pi-worker/src/main.ts`), bootstrapped
//!   content-`Resolved`, skipped with a printed reason when `bun` or the worker
//!   sources are absent.
//!
//! The framing here deliberately mirrors
//! `crates/lanes/ironclaw_sandbox/src/sandbox_process/loop_worker.rs` (u32
//! big-endian length prefix + JSON, same 1 MiB ceiling) without depending on
//! `ironclaw_sandbox`: the conformance lane must exercise the worker wire, not
//! the Docker exec plumbing.

use std::process::Stdio;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_agent_loop::test_support::{
    MockAgentLoopDriverHost, MockHostCall, ScenarioScript, ScriptedCapabilityCall,
    ScriptedCapabilityOutcome, ScriptedModelResponse, test_run_context,
};
use ironclaw_host_api::process::{RuntimeProcessError, SandboxLoopWorkerSession};
use ironclaw_loop_contracts::{
    AgentLoopDriverRunRequest, AgentLoopHostError, AgentLoopHostErrorKind, LoopCancelReasonKind,
    LoopCancellationSignal, LoopExit, LoopMessageContentPort, LoopModelMessage,
    ResolvedModelMessage, ResolvedToolResult,
};
use ironclaw_loop_host::{
    HostCall, HostFrame, HostRequestFrame, LoopWorkerInvocation, LoopWorkerOutcome,
    LoopWorkerSettings, WorkerContentVisibility, WorkerFrame, serve_loop_worker,
};
use ironclaw_turn_runner::sandboxed_planned_driver::{LoopWorkerKind, PI_LOOP_WORKER_EXECUTABLE};
use ironclaw_turns::LoopMessageRef;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const WORKER_CANCELLATION_GRACE: std::time::Duration = std::time::Duration::from_secs(5);
const PI_WORKER_ENTRY: &str = "docker/sandbox/pi-worker/src/main.ts";
const RESOLVED_BODY: &str = "resolved host body for msg:user";

/// Local child-process implementation of the sandbox worker session: identical
/// u32 big-endian framing to the Docker exec session in `ironclaw_sandbox`.
struct LocalProcessLoopWorkerSession {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
}

impl LocalProcessLoopWorkerSession {
    fn spawn(command: &mut tokio::process::Command) -> Result<Self, RuntimeProcessError> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "loop worker process failed to start: {error}"
            ))
        })?;
        let stdin = child.stdin.take().ok_or_else(|| {
            RuntimeProcessError::ExecutionFailed("loop worker stdin unavailable".to_string())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            RuntimeProcessError::ExecutionFailed("loop worker stdout unavailable".to_string())
        })?;
        Ok(Self {
            child,
            stdin,
            stdout,
        })
    }
}

#[async_trait]
impl SandboxLoopWorkerSession for LocalProcessLoopWorkerSession {
    async fn send(&mut self, frame: Vec<u8>) -> Result<(), RuntimeProcessError> {
        let length = u32::try_from(frame.len()).map_err(|_| {
            RuntimeProcessError::ExecutionFailed(
                "loop worker frame length cannot be represented".to_string(),
            )
        })?;
        self.stdin
            .write_u32(length)
            .await
            .map_err(|error| pipe_error("write", error))?;
        self.stdin
            .write_all(&frame)
            .await
            .map_err(|error| pipe_error("write", error))?;
        self.stdin
            .flush()
            .await
            .map_err(|error| pipe_error("write", error))
    }

    async fn receive(&mut self) -> Result<Option<Vec<u8>>, RuntimeProcessError> {
        let mut header = [0_u8; 4];
        match self.stdout.read_exact(&mut header).await {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                let _ = self.child.wait().await;
                return Ok(None);
            }
            Err(error) => return Err(pipe_error("read", error)),
        }
        let length = usize::try_from(u32::from_be_bytes(header)).map_err(|_| {
            RuntimeProcessError::ExecutionFailed("loop worker frame length is invalid".to_string())
        })?;
        if length > ironclaw_loop_host::LOOP_WORKER_MAX_FRAME_BYTES {
            return Err(RuntimeProcessError::ExecutionFailed(
                "loop worker emitted an oversized frame".to_string(),
            ));
        }
        let mut frame = vec![0_u8; length];
        self.stdout
            .read_exact(&mut frame)
            .await
            .map_err(|error| pipe_error("read", error))?;
        Ok(Some(frame))
    }

    async fn terminate(&mut self) -> Result<(), RuntimeProcessError> {
        let _ = self.child.kill().await;
        Ok(())
    }
}

fn pipe_error(direction: &str, error: std::io::Error) -> RuntimeProcessError {
    RuntimeProcessError::ExecutionFailed(format!("loop worker pipe {direction} failed: {error}"))
}

/// Fake `LoopMessageContentPort` standing in for the thread-backed resolver:
/// returns a fixed body for the one ref the prompt bundle issues.
#[derive(Default)]
struct FakeLoopMessageContentPort {
    resolve_calls: Mutex<usize>,
}

#[async_trait]
impl LoopMessageContentPort for FakeLoopMessageContentPort {
    async fn resolve_message_content(
        &self,
        messages: Vec<LoopModelMessage>,
    ) -> Result<Vec<ResolvedModelMessage>, AgentLoopHostError> {
        *self.resolve_calls.lock().unwrap() += 1;
        Ok(messages
            .into_iter()
            .map(|message| ResolvedModelMessage {
                role: message.role.clone(),
                content_ref: message.content_ref.clone(),
                content: RESOLVED_BODY.to_string(),
                tool_result: Some(ResolvedToolResult {
                    provider_call_id: Some("call-fake-tool".to_string()),
                    content: "fake tool result body".to_string(),
                }),
            })
            .collect())
    }
}

/// Runs `serve_loop_worker` against a spawned worker process and returns the
/// normalized outcome plus the host double it scripted.
async fn serve_local_worker(
    command: &mut tokio::process::Command,
    visibility: WorkerContentVisibility,
    script: ScenarioScript,
    cancel_after_first_model_call: bool,
) -> Result<(LoopWorkerOutcome, Arc<MockAgentLoopDriverHost>, usize), AgentLoopHostError> {
    let mut context = test_run_context("loop-worker-conformance");
    context.resolved_run_profile.loop_driver =
        ironclaw_turn_runner::planned_driver_factory::planned_driver_descriptor()
            .map_err(|reason| AgentLoopHostError::new(AgentLoopHostErrorKind::Internal, reason))?;

    let mut builder = MockAgentLoopDriverHost::builder()
        .run_context(context.clone())
        .script(script);
    if cancel_after_first_model_call {
        builder = builder.cancellation_signal(LoopCancellationSignal {
            reason_kind: LoopCancelReasonKind::UserRequested,
            requested_at: Utc::now(),
        });
    }
    let (host, _checkpoints) = builder.build();
    let host = Arc::new(host);

    let session = LocalProcessLoopWorkerSession::spawn(command).map_err(|error| {
        AgentLoopHostError::new(AgentLoopHostErrorKind::Unavailable, error.to_string())
    })?;
    let content = Arc::new(FakeLoopMessageContentPort::default());
    let invocation = LoopWorkerInvocation::Run(AgentLoopDriverRunRequest {
        turn_id: context.turn_id,
        run_id: context.run_id,
        resolved_run_profile: context.resolved_run_profile,
    });
    let settings = LoopWorkerSettings::default();

    let serve_host = Arc::clone(&host);
    let serve_visibility = visibility;
    let resolve_counter = Arc::clone(&content);
    let serve = tokio::spawn(async move {
        let content = Arc::clone(&content);
        let serve_content: Option<&dyn LoopMessageContentPort> = match serve_visibility {
            WorkerContentVisibility::Resolved => Some(&*content as &dyn LoopMessageContentPort),
            WorkerContentVisibility::Blind => None,
        };
        let mut session: Box<dyn SandboxLoopWorkerSession> = Box::new(session);
        serve_loop_worker(
            session.as_mut(),
            serve_host.as_ref(),
            invocation,
            settings,
            serve_content,
            serve_visibility,
        )
        .await
    });

    let result = tokio::time::timeout(WORKER_CANCELLATION_GRACE, serve)
        .await
        .map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "loop worker conformance scenario exceeded its grace window",
            )
        })?
        .map_err(|error| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                format!("serve_loop_worker task panicked: {error}"),
            )
        })?;
    let resolve_calls = *resolve_counter.resolve_calls.lock().unwrap();
    result.map(|outcome| (outcome, host, resolve_calls))
}

fn happy_path_script() -> ScenarioScript {
    ScenarioScript {
        model_responses: std::collections::VecDeque::from([
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo")]),
            ScriptedModelResponse::Reply {
                text: "conformance final reply".to_string(),
            },
        ]),
        capability_outcomes: std::collections::VecDeque::from([vec![
            ScriptedCapabilityOutcome::completed("result:conformance"),
        ]]),
        single_call_retry_outcomes: std::collections::VecDeque::from([
            ScriptedCapabilityOutcome::completed("result:conformance"),
        ]),
        pending_inputs: std::collections::VecDeque::new(),
    }
}

fn assert_completed_exit(outcome: &LoopWorkerOutcome) {
    match outcome {
        LoopWorkerOutcome::Exit(LoopExit::Completed(_)) => {}
        other => panic!("expected a completed loop exit, got {other:?}"),
    }
}

fn assert_cancelled_exit(outcome: &LoopWorkerOutcome) {
    match outcome {
        LoopWorkerOutcome::Exit(LoopExit::Cancelled(_)) => {}
        other => panic!("expected a cancelled loop exit, got {other:?}"),
    }
}

/// The HostCall sequence every conforming worker must produce: prompt bundle
/// before the first model call, one capability invocation between the two model
/// calls, transcript finalization after the final reply.
fn assert_happy_path_call_sequence(calls: &[MockHostCall], expected_model_calls: usize) {
    assert_eq!(
        calls
            .iter()
            .filter(|call| matches!(call, MockHostCall::StreamModel))
            .count(),
        expected_model_calls,
        "model call count must match the scripted scenario"
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| matches!(
                call,
                MockHostCall::InvokeCapabilityBatch { .. } | MockHostCall::InvokeCapability { .. }
            ))
            .count(),
        1,
        "exactly one capability invocation must cross the host port"
    );
    let build_prompt = calls
        .iter()
        .position(|call| matches!(call, MockHostCall::BuildPromptBundle))
        .unwrap_or_else(|| panic!("prompt bundle must be built before the first model call"));
    let first_model = calls
        .iter()
        .position(|call| matches!(call, MockHostCall::StreamModel))
        .expect("at least one model call must be recorded");
    assert!(
        build_prompt < first_model,
        "BuildPromptBundle must precede the first StreamModel"
    );
    let invoke = calls
        .iter()
        .position(|call| {
            matches!(
                call,
                MockHostCall::InvokeCapabilityBatch { .. } | MockHostCall::InvokeCapability { .. }
            )
        })
        .expect("capability invocation must be recorded");
    let second_model = calls
        .iter()
        .filter(|call| matches!(call, MockHostCall::StreamModel))
        .count();
    if second_model > 1 {
        let append = calls
            .iter()
            .position(|call| matches!(call, MockHostCall::AppendCapabilityResultRef { .. }))
            .expect("capability result evidence must be appended");
        let second_model_position = calls
            .iter()
            .enumerate()
            .filter(|(_, call)| matches!(call, MockHostCall::StreamModel))
            .nth(1)
            .map(|(index, _)| index)
            .expect("second model call must exist");
        assert!(
            invoke < append && append < second_model_position,
            "capability invocation, result append, and second model call must be ordered"
        );
    }
    assert!(
        calls
            .iter()
            .any(|call| matches!(call, MockHostCall::FinalizeAssistantMessage)),
        "final reply must be finalized through the transcript port"
    );
}

fn assert_checkpoint_staged(calls: &[MockHostCall]) {
    assert!(
        calls
            .iter()
            .any(|call| matches!(call, MockHostCall::StageCheckpointPayload(_))),
        "at least one checkpoint payload must be staged"
    );
    assert!(
        calls
            .iter()
            .any(|call| matches!(call, MockHostCall::SaveCheckpoint(_))),
        "at least one checkpoint metadata write must be recorded"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rust_worker_completes_the_scripted_happy_path_blind() {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_ironclaw-loop-worker"));
    let (outcome, host, resolve_calls) = serve_local_worker(
        &mut command,
        WorkerContentVisibility::Blind,
        happy_path_script(),
        false,
    )
    .await
    .expect("Rust worker happy-path scenario completes");

    assert_completed_exit(&outcome);
    let calls = host.call_log();
    assert_happy_path_call_sequence(&calls, 2);
    assert_checkpoint_staged(&calls);
    assert_eq!(
        resolve_calls, 0,
        "Blind lane must not resolve message content"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rust_worker_returns_cancelled_exit_within_the_grace_window() {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_ironclaw-loop-worker"));
    let (outcome, host, _) = serve_local_worker(
        &mut command,
        WorkerContentVisibility::Blind,
        happy_path_script(),
        true,
    )
    .await
    .expect("Rust worker cancellation scenario completes within the grace window");

    assert_cancelled_exit(&outcome);
    assert!(
        host.model_call_count() <= 1,
        "cancellation during the first model call must not reach a second model call"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pi_worker_completes_the_scripted_happy_path_resolved() {
    let Some(mut command) = pi_worker_command() else {
        eprintln!("SKIP: pi lane requires `bun` on PATH and {PI_WORKER_ENTRY} to exist");
        return;
    };
    let (outcome, host, resolve_calls) = serve_local_worker(
        &mut command,
        WorkerContentVisibility::Resolved,
        happy_path_script(),
        false,
    )
    .await
    .expect("Pi worker happy-path scenario completes");

    assert_completed_exit(&outcome);
    let calls = host.call_log();
    assert_happy_path_call_sequence(&calls, 2);
    assert_checkpoint_staged(&calls);
    assert!(
        resolve_calls >= 1,
        "Resolved Pi lane must resolve message content for its prompt bundle"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pi_worker_returns_cancelled_exit_within_the_grace_window() {
    let Some(mut command) = pi_worker_command() else {
        eprintln!("SKIP: pi lane requires `bun` on PATH and {PI_WORKER_ENTRY} to exist");
        return;
    };
    let (outcome, host, _) = serve_local_worker(
        &mut command,
        WorkerContentVisibility::Resolved,
        happy_path_script(),
        true,
    )
    .await
    .expect("Pi worker cancellation scenario completes within the grace window");

    assert_cancelled_exit(&outcome);
    assert!(
        host.model_call_count() <= 1,
        "cancellation during the first model call must not reach a second model call"
    );
}

/// Builds the Pi worker launch command, or `None` (with a printed skip reason)
/// when `bun` is not on PATH or the PiWorker sources are absent.
fn pi_worker_command() -> Option<tokio::process::Command> {
    let repo_root = env!("CARGO_MANIFEST_DIR");
    let entry = std::path::Path::new(repo_root)
        .join("../../..")
        .join(PI_WORKER_ENTRY);
    if !entry.is_file() {
        return None;
    }
    let bun = which_bun()?;
    let mut command = tokio::process::Command::new(bun);
    command.current_dir(std::path::Path::new(repo_root).join("../../.."));
    command.arg("run").arg(entry);
    Some(command)
}

fn which_bun() -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join("bun");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Pins the worker-kind executable selection the sandboxed driver performs:
/// Rust and Pi must launch distinct paths and map to their wire visibility.
#[test]
fn worker_kind_selects_executable_and_wire_visibility() {
    assert_eq!(
        LoopWorkerKind::Rust.executable(),
        "/usr/local/bin/ironclaw-loop-worker"
    );
    assert_eq!(LoopWorkerKind::Pi.executable(), PI_LOOP_WORKER_EXECUTABLE);
    assert_eq!(
        LoopWorkerKind::Rust.content_visibility(),
        WorkerContentVisibility::Blind
    );
    assert_eq!(
        LoopWorkerKind::Pi.content_visibility(),
        WorkerContentVisibility::Resolved
    );
    assert_eq!(LoopWorkerKind::parse("rust"), Some(LoopWorkerKind::Rust));
    assert_eq!(LoopWorkerKind::parse("PI"), Some(LoopWorkerKind::Pi));
    assert_eq!(LoopWorkerKind::parse("unsupported"), None);
}

/// A `ResolveMessages` call from a blind worker must be denied at the host:
/// the scripted double is the host here, and the denial is enforced by
/// `serve_loop_worker` dispatch. This test pins the host-side gate directly so
/// the Rust worker cannot regress into content visibility without failing the
/// blind lane.
#[tokio::test]
async fn blind_workers_cannot_resolve_message_content() {
    let context = test_run_context("loop-worker-conformance-blind");
    let content = Arc::new(FakeLoopMessageContentPort::default());
    let resolve_counter = Arc::clone(&content);
    let (host, _checkpoints) = MockAgentLoopDriverHost::builder()
        .run_context(context.clone())
        .build();
    // Blind dispatch: serve_loop_worker's dispatcher rejects ResolveMessages
    // before reaching the port. We drive the dispatch through a minimal
    // in-memory session that answers a single ResolveMessages request.
    let (mut host_session, mut worker_session) = duplex_pair();
    let host_side = tokio::spawn(async move {
        let bootstrap = LoopWorkerInvocation::Run(AgentLoopDriverRunRequest {
            turn_id: context.turn_id,
            run_id: context.run_id,
            resolved_run_profile: context.resolved_run_profile,
        });
        serve_loop_worker(
            &mut host_session,
            &host,
            bootstrap,
            LoopWorkerSettings::default(),
            Some(&*content as &dyn LoopMessageContentPort),
            WorkerContentVisibility::Blind,
        )
        .await
    });
    // Worker side: read the bootstrap, then issue one ResolveMessages call and
    // expect a PolicyDenied error response.
    let bootstrap = read_bootstrap_frame(&mut worker_session).await;
    assert_eq!(bootstrap.content_visibility, WorkerContentVisibility::Blind);
    let request = HostRequestFrame {
        id: 1,
        call: HostCall::ResolveMessages(ironclaw_loop_host::ResolveMessagesRequest {
            messages: vec![LoopModelMessage {
                role: "user".to_string(),
                content_ref: LoopMessageRef::new("msg:user").expect("valid test ref"),
            }],
        }),
    };
    write_framed(
        &worker_session,
        &serde_json::to_vec(&WorkerFrame::HostRequest(Box::new(request)))
            .expect("frame serializes"),
    )
    .await;
    let response = read_framed(&mut worker_session)
        .await
        .expect("host must respond");
    let frame: HostFrame = serde_json::from_slice(&response).expect("host frame decodes");
    match frame {
        HostFrame::HostResponse(response) => {
            let error = response
                .result
                .expect_err("blind ResolveMessages must be denied");
            let ironclaw_loop_host::WireError::Host(error) = error else {
                panic!("expected a host error, got {error:?}");
            };
            assert_eq!(error.kind, AgentLoopHostErrorKind::PolicyDenied);
        }
        other => panic!("expected a host response frame, got {other:?}"),
    }
    // serve_loop_worker keeps waiting for the outcome after answering; drop
    // the host side rather than waiting for an exit that never comes.
    host_side.abort();
    let _ = host_side.await;
    assert_eq!(
        *resolve_counter.resolve_calls.lock().unwrap(),
        0,
        "the content port must never be reached for a blind worker"
    );
}

fn duplex_pair() -> (TestHostSession, TestWorkerSession) {
    let host_to_worker = Arc::new(Mutex::new(Vec::new()));
    let worker_to_host = Arc::new(Mutex::new(Vec::new()));
    (
        TestHostSession {
            input: Arc::clone(&host_to_worker),
            output: Arc::clone(&worker_to_host),
        },
        TestWorkerSession {
            host_to_worker,
            worker_to_host,
        },
    )
}

struct TestHostSession {
    input: Arc<Mutex<Vec<u8>>>,
    output: Arc<Mutex<Vec<u8>>>,
}

#[async_trait]
impl SandboxLoopWorkerSession for TestHostSession {
    async fn send(&mut self, frame: Vec<u8>) -> Result<(), RuntimeProcessError> {
        let length = u32::try_from(frame.len()).map_err(|_| {
            RuntimeProcessError::ExecutionFailed("frame length cannot be represented".to_string())
        })?;
        let mut buffer = length.to_be_bytes().to_vec();
        buffer.extend_from_slice(&frame);
        self.input.lock().unwrap().extend_from_slice(&buffer);
        Ok(())
    }

    async fn receive(&mut self) -> Result<Option<Vec<u8>>, RuntimeProcessError> {
        loop {
            let frame = {
                let mut buffer = self.output.lock().unwrap();
                if buffer.len() >= 4 {
                    let length = usize::try_from(u32::from_be_bytes([
                        buffer[0], buffer[1], buffer[2], buffer[3],
                    ]))
                    .map_err(|_| {
                        RuntimeProcessError::ExecutionFailed("frame length is invalid".to_string())
                    })?;
                    if buffer.len() >= 4 + length {
                        buffer.drain(..4);
                        Some(buffer.drain(..length).collect())
                    } else {
                        None
                    }
                } else {
                    None
                }
            };
            if let Some(frame) = frame {
                return Ok(Some(frame));
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    }

    async fn terminate(&mut self) -> Result<(), RuntimeProcessError> {
        Ok(())
    }
}

/// Test-side accessor over the same pipes: reads bootstrap frames and writes
/// worker frames exactly as a real worker would over stdio.
struct TestWorkerSession {
    host_to_worker: Arc<Mutex<Vec<u8>>>,
    worker_to_host: Arc<Mutex<Vec<u8>>>,
}

impl TestWorkerSession {
    async fn read(&self) -> Option<Vec<u8>> {
        loop {
            {
                let mut buffer = self.host_to_worker.lock().unwrap();
                if buffer.len() >= 4 {
                    let length = usize::try_from(u32::from_be_bytes([
                        buffer[0], buffer[1], buffer[2], buffer[3],
                    ]))
                    .expect("in-memory test framing stays bounded");
                    if buffer.len() >= 4 + length {
                        buffer.drain(..4);
                        return Some(buffer.drain(..length).collect());
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    }

    async fn write(&self, bytes: &[u8]) {
        let length = u32::try_from(bytes.len()).expect("test frame stays bounded");
        let mut buffer = length.to_be_bytes().to_vec();
        buffer.extend_from_slice(bytes);
        self.worker_to_host
            .lock()
            .unwrap()
            .extend_from_slice(&buffer);
    }
}

async fn read_bootstrap_frame(
    worker: &mut TestWorkerSession,
) -> ironclaw_loop_host::LoopWorkerBootstrap {
    let bytes = worker.read().await.expect("bootstrap frame arrives");
    let frame: HostFrame = serde_json::from_slice(&bytes).expect("bootstrap frame decodes");
    match frame {
        HostFrame::Bootstrap(bootstrap) => *bootstrap,
        other => panic!("expected a bootstrap frame, got {other:?}"),
    }
}

async fn write_framed(worker: &TestWorkerSession, bytes: &[u8]) {
    worker.write(bytes).await;
}

async fn read_framed(worker: &mut TestWorkerSession) -> Option<Vec<u8>> {
    worker.read().await
}
