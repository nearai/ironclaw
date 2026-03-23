# Docker Detection Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add proactive Docker detection at startup and in the setup wizard, with platform-specific installation guidance.

**Architecture:** New `src/sandbox/detect.rs` module for centralized Docker detection. Wizard gets a new step. Startup check in `main.rs` warns and disables sandbox if Docker unavailable. Boot screen shows Docker status.

**Tech Stack:** Rust, bollard (existing), std::process::Command

---

### Task 1: Create `src/sandbox/detect.rs` -- Docker Detection Module

**Files:**
- Create: `src/sandbox/detect.rs`
- Modify: `src/sandbox/mod.rs`

**Step 1: Write the failing test**

```rust
// In src/sandbox/detect.rs

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_platform() {
        let platform = Platform::current();
        // Should return a valid platform on any CI/dev machine
        match platform {
            Platform::MacOS | Platform::Linux | Platform::Windows => {}
        }
    }

    #[test]
    fn test_install_hint_not_empty() {
        for platform in [Platform::MacOS, Platform::Linux, Platform::Windows] {
            assert!(!platform.install_hint().is_empty());
            assert!(!platform.start_hint().is_empty());
        }
    }

    #[test]
    fn test_docker_status_display() {
        assert_eq!(DockerStatus::Available.as_str(), "available");
        assert_eq!(DockerStatus::NotInstalled.as_str(), "not installed");
        assert_eq!(DockerStatus::NotRunning.as_str(), "not running");
        assert_eq!(DockerStatus::Disabled.as_str(), "disabled");
    }

    #[test]
    fn test_docker_status_is_ok() {
        assert!(DockerStatus::Available.is_ok());
        assert!(!DockerStatus::NotInstalled.is_ok());
        assert!(!DockerStatus::NotRunning.is_ok());
        assert!(!DockerStatus::Disabled.is_ok());
    }

    #[tokio::test]
    async fn test_check_docker_returns_valid_status() {
        let result = check_docker().await;
        // On CI without Docker, should be NotInstalled or NotRunning
        // On dev with Docker, should be Available
        // Either way, should not panic
        match result.status {
            DockerStatus::Available
            | DockerStatus::NotInstalled
            | DockerStatus::NotRunning => {}
            DockerStatus::Disabled => panic!("check_docker should never return Disabled"),
        }
    }
}
```

**Step 2: Write the implementation**

```rust
//! Proactive Docker detection with platform-specific guidance.

/// Docker daemon availability status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockerStatus {
    /// Docker binary found on PATH and daemon responding to ping.
    Available,
    /// `docker` binary not found on PATH.
    NotInstalled,
    /// Binary found but daemon not responding.
    NotRunning,
    /// Sandbox feature not enabled (no check performed).
    Disabled,
}

impl DockerStatus {
    pub fn is_ok(&self) -> bool {
        matches!(self, DockerStatus::Available)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            DockerStatus::Available => "available",
            DockerStatus::NotInstalled => "not installed",
            DockerStatus::NotRunning => "not running",
            DockerStatus::Disabled => "disabled",
        }
    }
}

/// Host platform for install guidance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    MacOS,
    Linux,
    Windows,
}

impl Platform {
    pub fn current() -> Self {
        match std::env::consts::OS {
            "macos" => Platform::MacOS,
            "windows" => Platform::Windows,
            _ => Platform::Linux,
        }
    }

    pub fn install_hint(&self) -> &'static str {
        match self {
            Platform::MacOS => "Install Docker Desktop: https://docs.docker.com/desktop/install/mac-install/",
            Platform::Linux => "Install Docker Engine: https://docs.docker.com/engine/install/",
            Platform::Windows => "Install Docker Desktop: https://docs.docker.com/desktop/install/windows-install/",
        }
    }

    pub fn start_hint(&self) -> &'static str {
        match self {
            Platform::MacOS => "Start Docker Desktop from Applications, or run: open -a Docker",
            Platform::Linux => "Start the Docker daemon: sudo systemctl start docker",
            Platform::Windows => "Start Docker Desktop from the Start menu",
        }
    }
}

/// Result of a Docker detection check.
pub struct DockerDetection {
    pub status: DockerStatus,
    pub platform: Platform,
}

/// Check whether Docker is installed and running.
///
/// 1. Checks if `docker` binary exists on PATH
/// 2. If found, tries to connect and ping the Docker daemon
/// 3. Returns `Available`, `NotInstalled`, or `NotRunning`
pub async fn check_docker() -> DockerDetection {
    let platform = Platform::current();

    // Step 1: Check if docker binary is on PATH
    if !docker_binary_exists() {
        return DockerDetection {
            status: DockerStatus::NotInstalled,
            platform,
        };
    }

    // Step 2: Try to connect to the daemon
    match crate::sandbox::connect_docker().await {
        Ok(_) => DockerDetection {
            status: DockerStatus::Available,
            platform,
        },
        Err(_) => DockerDetection {
            status: DockerStatus::NotRunning,
            platform,
        },
    }
}

/// Check if the `docker` binary exists on PATH.
fn docker_binary_exists() -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("which")
            .arg("docker")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
    #[cfg(windows)]
    {
        std::process::Command::new("where")
            .arg("docker")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
}
```

**Step 3: Export from `src/sandbox/mod.rs`**

Add `pub mod detect;` and re-export key types.

**Step 4: Run tests**

Run: `cargo test sandbox::detect::tests -- --nocapture`
Expected: All pass

**Step 5: Clippy**

Run: `cargo clippy --all --all-features`
Expected: Zero warnings on new code

**Step 6: Commit**

```bash
git add src/sandbox/detect.rs src/sandbox/mod.rs
git commit -m "feat: add Docker detection module with platform guidance"
```

---

### Task 2: Update Boot Screen to Show Docker Status

**Files:**
- Modify: `src/boot_screen.rs`

**Step 1: Add `docker_status` to `BootInfo`**

Add field: `pub docker_status: DockerStatus` (import from `crate::sandbox::detect::DockerStatus`).

**Step 2: Update `print_boot_screen` features rendering**

When sandbox is enabled in config but Docker isn't available, show a warning:
- `DockerStatus::Available`: "sandbox" (as today)
- `DockerStatus::NotInstalled`: "sandbox (docker not installed)" in yellow
- `DockerStatus::NotRunning`: "sandbox (docker not running)" in yellow
- `DockerStatus::Disabled`: don't show sandbox (as today)

**Step 3: Update tests**

Update all 3 existing `BootInfo` test structs to include `docker_status` field.

**Step 4: Run tests**

Run: `cargo test boot_screen::tests`
Expected: All pass

**Step 5: Commit**

```bash
git add src/boot_screen.rs
git commit -m "feat: show Docker status in boot screen"
```

---

### Task 3: Add Startup Docker Check in `main.rs`

**Files:**
- Modify: `src/main.rs`

**Step 1: Add Docker check before `ContainerJobManager` creation**

Before line ~989 (`let container_job_manager = if config.sandbox.enabled`), insert:

```rust
// Proactive Docker detection
let docker_status = if config.sandbox.enabled {
    let detection = ironclaw::sandbox::detect::check_docker().await;
    match detection.status {
        ironclaw::sandbox::detect::DockerStatus::Available => {
            tracing::info!("Docker is available");
            detection.status
        }
        ironclaw::sandbox::detect::DockerStatus::NotInstalled => {
            tracing::warn!(
                "Docker is not installed. Sandbox disabled for this session. {}",
                detection.platform.install_hint()
            );
            detection.status
        }
        ironclaw::sandbox::detect::DockerStatus::NotRunning => {
            tracing::warn!(
                "Docker is installed but not running. Sandbox disabled for this session. {}",
                detection.platform.start_hint()
            );
            detection.status
        }
        ironclaw::sandbox::detect::DockerStatus::Disabled => detection.status,
    }
} else {
    ironclaw::sandbox::detect::DockerStatus::Disabled
};
```

Then gate the `ContainerJobManager` creation on `docker_status.is_ok()`:
```rust
let container_job_manager = if config.sandbox.enabled && docker_status.is_ok() {
    // ... existing code ...
```

**Step 2: Pass `docker_status` to `BootInfo`**

In the boot screen construction, add the `docker_status` field.

**Step 3: Run full test suite**

Run: `cargo test`
Expected: All pass

**Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat: check Docker availability at startup"
```

---

### Task 4: Add Docker/Sandbox Wizard Step

**Files:**
- Modify: `src/setup/wizard.rs`

**Step 1: Increment `total_steps` from 8 to 9**

**Step 2: Add `step_docker_sandbox()` method**

Insert after Extensions (step 7), before Heartbeat:

```rust
/// Step 8: Docker Sandbox
async fn step_docker_sandbox(&mut self) -> Result<(), SetupError> {
    print_info("The Docker sandbox provides isolated execution for code generation,");
    print_info("builds, and untrusted commands. It requires Docker to be installed.");
    println!();

    if !confirm("Enable Docker sandbox?", false).map_err(SetupError::Io)? {
        self.settings.sandbox.enabled = false;
        print_info("Sandbox disabled. You can enable it later with SANDBOX_ENABLED=true.");
        return Ok(());
    }

    // Check Docker availability
    let detection = crate::sandbox::detect::check_docker().await;

    match detection.status {
        crate::sandbox::detect::DockerStatus::Available => {
            self.settings.sandbox.enabled = true;
            print_success("Docker is installed and running. Sandbox enabled.");
        }
        crate::sandbox::detect::DockerStatus::NotInstalled => {
            println!();
            print_error("Docker is not installed.");
            print_info(detection.platform.install_hint());
            println!();
            // Offer retry or skip
            if confirm("Retry after installing Docker?", false).map_err(SetupError::Io)? {
                let retry = crate::sandbox::detect::check_docker().await;
                if retry.status.is_ok() {
                    self.settings.sandbox.enabled = true;
                    print_success("Docker is now available. Sandbox enabled.");
                } else {
                    self.settings.sandbox.enabled = false;
                    print_info("Docker still not available. Sandbox disabled for now.");
                }
            } else {
                self.settings.sandbox.enabled = false;
                print_info("Sandbox disabled. Install Docker and set SANDBOX_ENABLED=true later.");
            }
        }
        crate::sandbox::detect::DockerStatus::NotRunning => {
            println!();
            print_error("Docker is installed but not running.");
            print_info(detection.platform.start_hint());
            println!();
            if confirm("Retry after starting Docker?", false).map_err(SetupError::Io)? {
                let retry = crate::sandbox::detect::check_docker().await;
                if retry.status.is_ok() {
                    self.settings.sandbox.enabled = true;
                    print_success("Docker is now running. Sandbox enabled.");
                } else {
                    self.settings.sandbox.enabled = false;
                    print_info("Docker still not responding. Sandbox disabled for now.");
                }
            } else {
                self.settings.sandbox.enabled = false;
                print_info("Sandbox disabled. Start Docker and set SANDBOX_ENABLED=true later.");
            }
        }
        _ => {
            self.settings.sandbox.enabled = false;
        }
    }

    Ok(())
}
```

**Step 3: Wire into `run()` method**

```rust
// Step 8: Docker Sandbox
print_step(8, total_steps, "Docker Sandbox");
self.step_docker_sandbox().await?;
self.persist_after_step().await;

// Step 9: Heartbeat (was Step 8)
print_step(9, total_steps, "Background Tasks");
self.step_heartbeat()?;
self.persist_after_step().await;
```

**Step 4: Run tests**

Run: `cargo test setup`
Expected: All pass

**Step 5: Commit**

```bash
git add src/setup/wizard.rs
git commit -m "feat: add Docker sandbox step to setup wizard"
```

---

### Task 5: Final Verification

**Step 1: Run full test suite**

Run: `cargo test`
Expected: All pass

**Step 2: Run clippy**

Run: `cargo clippy --all --all-features --benches --tests --examples`
Expected: Zero warnings

**Step 3: Check for unwrap/expect in production code**

Grep changed files for `.unwrap()` and `.expect(` -- should have none in production code.

**Step 4: Verify both feature flags compile**

Run: `cargo check` and `cargo check --no-default-features --features libsql`
Expected: Both clean
