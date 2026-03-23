# Proactive Docker Detection

Date: 2026-02-21

## Problem

IronClaw's sandbox system requires Docker but provides no proactive guidance. Docker availability is only checked at runtime when a sandbox job is attempted, resulting in a confusing error. Users have no way to know during setup or startup whether Docker is properly configured.

## Goals

1. Detect Docker installation AND daemon running status at two points: setup wizard and every startup
2. Provide platform-specific installation guidance (macOS, Linux, Windows)
3. Surface Docker status clearly in the boot screen
4. Allow users to skip/continue without Docker (sandbox is optional)

## Non-Goals

- Auto-installing Docker
- Changing the default sandbox setting (stays `enabled: false`)
- Modifying the existing `connect_docker()` function

## Design

### Docker Status Model

New file `src/sandbox/detect.rs` with centralized detection:

```rust
pub enum DockerStatus {
    Available,      // Binary on PATH + daemon responding to ping
    NotInstalled,   // `docker` binary not found on PATH
    NotRunning,     // Binary found but daemon not responding
    Disabled,       // Sandbox not enabled (no check performed)
}

pub enum Platform { MacOS, Linux, Windows }

pub struct DockerDetection {
    pub status: DockerStatus,
    pub platform: Platform,
}
```

Detection logic:
1. Check if `docker` binary exists on PATH (reuse `which`/`where` pattern from `skills/gating.rs`)
2. If found, attempt `connect_docker()` to ping the daemon
3. Return `Available`, `NotInstalled`, or `NotRunning`

### Platform-Specific Guidance

| Platform | Not Installed | Not Running |
|----------|--------------|-------------|
| macOS | "Install Docker Desktop: https://docs.docker.com/desktop/install/mac-install/" | "Start Docker Desktop from Applications, or run: open -a Docker" |
| Linux | "Install Docker Engine: https://docs.docker.com/engine/install/" | "Start the Docker daemon: sudo systemctl start docker" |
| Windows | "Install Docker Desktop: https://docs.docker.com/desktop/install/windows-install/" | "Start Docker Desktop from the Start menu" |

### Wizard Step (First-Run)

Add Step 8 "Docker Sandbox" (current steps 8 becomes 9, total becomes 9):

1. Ask "Do you want to enable Docker sandbox for isolated code execution?"
2. If yes, run Docker detection
3. Based on status:
   - **Available**: Enable sandbox, confirm
   - **Not Installed**: Show install guidance, offer to skip or retry after installing
   - **Not Running**: Show start guidance, offer to skip or retry
4. If user skips, sandbox stays disabled

### Startup Check (Every Launch)

In `main.rs`, when `config.sandbox.enabled == true`, before creating `ContainerJobManager`:

1. Run `DockerDetection::check()`
2. If **Available**: proceed normally
3. If **NotInstalled** or **NotRunning**: log warning, disable sandbox for this session, continue startup

### Boot Screen Changes

`BootInfo` gains `docker_status: DockerStatus` field.

Features line rendering:
- `Available` + enabled: `sandbox` (as today)
- `NotInstalled` + enabled in config: `sandbox (docker not installed)`
- `NotRunning` + enabled in config: `sandbox (docker not running)`
- `Disabled`: no sandbox shown (as today)

Warning lines shown in yellow when Docker is configured but unavailable.

## Files

| Action | File | Change |
|--------|------|--------|
| Create | `src/sandbox/detect.rs` | Detection logic, platform hints |
| Modify | `src/sandbox/mod.rs` | Export `detect` module |
| Modify | `src/setup/wizard.rs` | Add Docker/Sandbox wizard step |
| Modify | `src/main.rs` | Startup check before ContainerJobManager |
| Modify | `src/boot_screen.rs` | Show Docker status |

## Dependencies

No new crate dependencies. Uses existing `bollard` (via `connect_docker()`), `std::process::Command` (for binary detection), and `std::env::consts::OS` (for platform detection).
