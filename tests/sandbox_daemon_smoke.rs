//! Smoke tests for the standalone `sandbox_daemon` binary.
//!
//! These tests build the binary via `env!("CARGO_BIN_EXE_sandbox_daemon")`
//! (provided automatically by Cargo for any `[[bin]]` target) and drive it
//! by piping NDJSON to stdin and reading NDJSON from stdout. They cover the
//! protocol surface end-to-end without spinning up a real container — that
//! way Phase 5's host-side `ContainerizedFilesystemBackend` can swap in
//! the same binary inside Docker without protocol drift.

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn run_daemon(base_dir: &std::path::Path, lines: &[Value]) -> Vec<Value> {
    let bin = env!("CARGO_BIN_EXE_sandbox_daemon");
    let mut child = Command::new(bin)
        .env("IRONCLAW_SANDBOX_BASE_DIR", base_dir)
        .env("IRONCLAW_SANDBOX_LOG", "warn")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn sandbox_daemon");

    {
        let stdin = child.stdin.as_mut().expect("stdin");
        for v in lines {
            let mut bytes = serde_json::to_vec(v).expect("serialize request");
            bytes.push(b'\n');
            stdin.write_all(&bytes).expect("write to daemon stdin");
        }
        // Closing stdin via drop signals EOF if no shutdown was sent.
    }

    let output = child.wait_with_output().expect("daemon exit");
    assert!(
        output.status.success(),
        "daemon exited non-zero: {:?} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("utf-8 stdout");
    stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("parse response line"))
        .collect()
}

#[test]
fn health_returns_ok_with_tools() {
    let dir = tempfile::tempdir().unwrap();
    let resps = run_daemon(
        dir.path(),
        &[
            json!({"id": "1", "method": "health"}),
            json!({"id": "2", "method": "shutdown"}),
        ],
    );

    assert_eq!(resps.len(), 2);
    let health = &resps[0];
    assert_eq!(health["id"], "1");
    assert_eq!(health["result"]["status"], "ok");
    let tools = health["result"]["tools"].as_array().expect("tools array");
    let names: Vec<&str> = tools.iter().filter_map(|t| t.as_str()).collect();
    for expected in [
        "file_read",
        "file_write",
        "list_dir",
        "apply_patch",
        "shell",
    ] {
        assert!(
            names.contains(&expected),
            "health response missing tool: {expected}"
        );
    }
}

#[test]
fn write_then_read_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let resps = run_daemon(
        dir.path(),
        &[
            json!({
                "id": "w",
                "method": "execute_tool",
                "params": {
                    "name": "file_write",
                    "input": {"path": "hello.txt", "content": "world"}
                }
            }),
            json!({
                "id": "r",
                "method": "execute_tool",
                "params": {
                    "name": "file_read",
                    "input": {"path": "hello.txt"}
                }
            }),
            json!({"id": "x", "method": "shutdown"}),
        ],
    );

    assert_eq!(resps.len(), 3);
    assert_eq!(resps[0]["id"], "w");
    assert_eq!(resps[0]["result"]["output"]["bytes_written"], 5);
    assert!(dir.path().join("hello.txt").exists());

    assert_eq!(resps[1]["id"], "r");
    let content = resps[1]["result"]["output"]["content"]
        .as_str()
        .expect("content string");
    assert!(content.contains("world"));
}

#[test]
fn unknown_tool_returns_tool_error() {
    let dir = tempfile::tempdir().unwrap();
    let resps = run_daemon(
        dir.path(),
        &[
            json!({
                "id": "u",
                "method": "execute_tool",
                "params": {"name": "nope", "input": {}}
            }),
            json!({"id": "x", "method": "shutdown"}),
        ],
    );
    assert_eq!(resps[0]["id"], "u");
    let err = &resps[0]["error"];
    assert!(!err.is_null(), "expected error response, got: {resps:?}");
    assert_eq!(err["code"], "tool_error");
    assert!(err["message"].as_str().unwrap().contains("nope"));
}

#[test]
fn unknown_method_returns_unknown_method() {
    let dir = tempfile::tempdir().unwrap();
    let resps = run_daemon(
        dir.path(),
        &[
            json!({"id": "u", "method": "ride_unicorn"}),
            json!({"id": "x", "method": "shutdown"}),
        ],
    );
    assert_eq!(resps[0]["error"]["code"], "unknown_method");
}

#[test]
fn malformed_json_returns_parse_error() {
    use std::io::Read;
    let bin = env!("CARGO_BIN_EXE_sandbox_daemon");
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(bin)
        .env("IRONCLAW_SANDBOX_BASE_DIR", dir.path())
        .env("IRONCLAW_SANDBOX_LOG", "warn")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn sandbox_daemon");
    {
        let stdin = child.stdin.as_mut().unwrap();
        writeln!(stdin, "{{not json").unwrap();
        writeln!(stdin, "{{\"id\":\"x\",\"method\":\"shutdown\"}}").unwrap();
    }
    let mut out = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    let _ = child.wait();
    let first = out.lines().next().unwrap();
    let parsed: Value = serde_json::from_str(first).unwrap();
    assert_eq!(parsed["error"]["code"], "parse_error");
}
