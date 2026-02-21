# IronClaw Documentation Audit Report

> **Report Generated**: 2026-02-21  
> **Analyst**: Matrix Agent  
> **Scope**: IronClaw source code (~/src/work/ironclaw) vs Ironclaw-docs (~/src/work/ironclaw-docs)

---

## Executive Summary

This report presents a comprehensive comparative analysis between the IronClaw source code and its official documentation. The audit reveals several significant discrepancies that could confuse developers and operators deploying or contributing to IronClaw.

### Health Score: **MEDIUM**

| Category | Status | Finding Count |
|----------|--------|---------------|
| Critical | 🔴 High | 3 |
| Major | 🟡 Medium | 5 |
| Minor | 🟢 Low | 7 |

### Top 3 Priority Issues

1. **Version Drift**: Documentation describes v0.7.0 but source is v0.9.0 — a 0.2.0 gap
2. **Undocumented Features**: Claude Bridge, Worker mode, and Memory commands missing from docs
3. **Configuration Inconsistency**: Several environment variables differ between docs and source

---

## 1. Methodology

The analysis compared the following:

- **Documentation**: ~/src/work/ironclaw-docs/
  - README.md
  - ARCHITECTURE.md
  - DEPLOYMENT.md
  - DEVELOPER-REFERENCE.md
  - analysis/ directory (11 detailed documents)

- **Source Code**: ~/src/work/ironclaw/
  - Cargo.toml (project metadata)
  - src/main.rs (entry point, ~1600 lines)
  - src/config/*.rs (configuration system)
  - Directory structure analysis

---

## 2. Critical Discrepancies

### 2.1 Version Mismatch

| Location | Version | File |
|----------|---------|------|
| Documentation | **v0.7.0** | ironclaw-docs/README.md:5 |
| Source Code | **v0.9.0** | Cargo.toml:6 |

**Impact**: High — All API references, feature lists, and configuration examples maybe outdated.

**Evidence**:
```
Documentation (README.md):
> Comprehensive developer reference for IronClaw v0.7.0

Source (Cargo.toml):
version = "0.9.0"
```

**Recommendation**: Update all documentation to reflect v0.9.0 or implement automated version synchronization.

---

### 2.2 File Count Discrepancy

| Metric | Documentation | Source Code | Difference |
|--------|--------------|-------------|------------|
| Tools module (.rs files) | 39 | 45+ | -6 |
| Source files (total) | 242 | Not verified | — |

**Analysis**:

The ARCHITECTURE.md claims 39 files in the tools/ module. Actual count:

| Subdirectory | Files |
|-------------|-------|
| tools/ (root) | 4 (mod.rs, tool.rs, registry.rs, rate_limiter.rs) |
| tools/builtin/ | 12 |
| tools/wasm/ | 13 |
| tools/builder/ | 4+ |
| tools/mcp/ | 2+ |
| **Total** | **45+** |

**Recommendation**: Update ARCHITECTURE.md Section 12 with accurate file counts.

---

### 2.3 Missing Undocumented Features

The source code contains features entirely absent from documentation:

#### 2.3.1 Claude Bridge Mode

**Evidence** (src/main.rs:244-290):
```rust
Some(Command::ClaudeBridge {
    job_id,
    orchestrator_url,
    max_turns,
    model,
}) => {
    // Claude Code bridge mode: runs inside a Docker container.
    // Spawns the `claude` CLI and streams output to the orchestrator.
    ...
    let runtime = ironclaw::worker::ClaudeBridgeRuntime::new(config)
```

**Documentation Status**: Not mentioned anywhere in ironclaw-docs.

**Impact**: Users cannot discover or use Claude Code integration.

---

#### 2.3.2 Worker Mode

**Evidence** (src/main.rs:213-243):
```rust
Some(Command::Worker {
    job_id,
    orchestrator_url,
    max_iterations,
}) => {
    // Worker mode: runs inside a Docker container.
    // Simple logging (no TUI, no DB, no channels).
    ...
```

**Documentation Status**: analysis/worker-orchestrator.md exists but doesn't document the Worker command.

---

#### 2.3.3 Memory Commands

**Evidence** (src/main.rs:89-160):
```rust
Some(Command::Memory(mem_cmd)) => {
    // Memory commands need database (and optionally embeddings)
    let config = Config::from_env().await...
```

**Documentation Status**: Not documented in any DEPLOYMENT or CLI reference.

---

## 3. Major Discrepancies

### 3.1 Configuration Variable Naming Inconsistencies

| Documentation (DEPLOYMENT.md) | Source Code (config/agent.rs) | Status |
|-------------------------------|------------------------------|--------|
| AGENT_MAX_PARALLEL_JOBS | AGENT_MAX_PARALLEL_JOBS | ✅ Matches |
| AGENT_JOB_TIMEOUT_SECS | AGENT_JOB_TIMEOUT_SECS | ✅ Matches |
| — | AGENT_MAX_TOOL_ITERATIONS | ❌ Missing |
| — | AGENT_AUTO_APPROVE_TOOLS | ❌ Missing |
| — | AGENT_USE_PLANNING | ❌ Missing |
| — | AGENT_STUCK_THRESHOLD_SECS | ❌ Missing |
| — | SESSION_IDLE_TIMEOUT_SECS | ❌ Missing |

**Impact**: Advanced users cannot configure these behaviors.

---

### 3.2 Build Command Differences

| Aspect | Documentation | Source Code |
|--------|--------------|-------------|
| Default features | postgres + libsql | postgres + libsql + html-to-markdown |
| Edition | 2021 | 2024 |
| Rust version | 1.92+ | 1.92+ |

**Evidence** (Cargo.toml:7):
```toml
edition = "2024"
```

**Note**: The documentation doesn't mention the `html-to-markdown` feature which is included in default.

---

### 3.3 Missing Analysis Documents

The documentation index (README.md) lists 14 analysis documents, but the workspace-memory.md document referenced doesn't exist in the analysis directory:

| Listed in README | Exists |
|-----------------|--------|
| analysis/agent.md | ✅ |
| analysis/channels.md | ✅ |
| analysis/cli.md | ✅ |
| analysis/config.md | ✅ |
| analysis/llm.md | ✅ |
| analysis/safety-sandbox.md | ✅ |
| analysis/secrets-keychain.md | ✅ |
| analysis/skills-extensions.md | ✅ |
| analysis/tools.md | ✅ |
| analysis/tunnels-pairing.md | ✅ |
| analysis/worker-orchestrator.md | ✅ |
| analysis/workspace-memory.md | ❌ Missing |

---

### 3.4 Rust Edition Mismatch in Documentation

DEPLOYMENT.md states:
> **Rust toolchain**: 1.92+ via `rustup` or Homebrew

Source Cargo.toml shows:
```toml
edition = "2024"
```

**Issue**: Rust Edition 2024 requires Rust 1.85+, not 1.92. This suggests the edition may have been updated recently but docs weren't synchronized.

---

### 3.5 Default Values Discrepancy

| Setting | Documentation | Source Code Default |
|---------|--------------|-------------------|
| GATEWAY_PORT | 3002 | 3000 (in config) |
| Database default | libsql | Varies by feature |

---

## 4. Minor Discrepancies

### 4.1 Binary Size

DEPLOYMENT.md claims:
> Binary size: ~49MB (release, macOS arm64)

**Note**: This wasn't verified in this audit but should be retested with v0.9.0.

---

### 4.2 Build Time

DEPLOYMENT.md claims:
> Build time: ~9 minutes cold, ~3 minutes incremental

**Note**: Should be retested with current hardware and v0.9.0.

---

### 4.3 Source Line Count

Documentation claims:
> ~107,000 lines of Rust

This was not independently verified but represents a significant codebase.

---

### 4.4 Documentation Generation Date

| Document | Stated Date |
|----------|-------------|
| ARCHITECTURE.md | 2026-02-21 (v0.7.0) |
| DEPLOYMENT.md | (No date, but references v0.7.0) |
| README.md | Generated: 2026-02-21 |

The dates are current but tied to v0.7.0.

---

### 4.5 REPL Implementation Clarification

Current source uses:
- `rustyline` + `termimad` for the interactive REPL (`src/channels/repl.rs`)
- no `ratatui` dependency in current `Cargo.toml`

**Status**: Clarified. Documentation now describes REPL mode (not a legacy TUI).

---

### 4.6 CLI Module Location

Current structure:
- `src/cli/` = CLI command routing/subcommands
- `src/channels/repl.rs` = interactive terminal channel
- no dedicated `channels/cli` directory in current source

**Status**: Clarified in architecture and developer reference docs.

---

### 4.7 Missing Config Module Documentation

Source config/ directory contains:
- agent.rs
- builder.rs
- channels.rs
- database.rs
- embeddings.rs
- heartbeat.rs
- helpers.rs
- hygiene.rs
- llm.rs
- mod.rs
- routines.rs
- safety.rs
- sandbox.rs
- secrets.rs
- skills.rs
- tunnel.rs
- wasm.rs

**Missing from docs**: hygiene.rs, builder.rs configuration sections

---

## 5. Issue Summary by Severity

### Critical (3)

| ID | Issue | Location | Recommendation |
|----|-------|----------|----------------|
| C1 | Version mismatch (v0.7.0 vs v0.9.0) | All docs | Update all docs to v0.9.0 |
| C2 | Undocumented Claude Bridge feature | Source only | Add documentation |
| C3 | Undocumented Worker mode | Source only | Add documentation |

### Major (5)

| ID | Issue | Location | Recommendation |
|----|-------|----------|----------------|
| M1 | File count discrepancy (39 vs 45+) | ARCHITECTURE.md | Update counts |
| M2 | Missing environment variables | DEPLOYMENT.md | Add AGENT_MAX_TOOL_ITERATIONS, etc. |
| M3 | Missing workspace-memory.md | analysis/ | Create or remove from index |
| M4 | Rust Edition confusion | DEPLOYMENT.md | Clarify edition vs version |
| M5 | Missing Memory commands | CLI docs | Document ironclaw memory command |

### Minor (7)

| ID | Issue | Location | Recommendation |
|----|-------|----------|----------------|
| N1 | Binary size outdated | DEPLOYMENT.md | Retest and update |
| N2 | Build time outdated | DEPLOYMENT.md | Retest and update |
| N3 | REPL implementation wording | ARCHITECTURE.md | Resolved (REPL wording updated) |
| N4 | CLI module location | ARCHITECTURE.md | Resolved (`src/cli/` + `src/channels/repl.rs`) |
| N5 | Missing hygiene.rs docs | Config docs | Add section |
| N6 | Missing builder.rs docs | Config docs | Add section |
| N7 | GATEWAY_PORT default | Docs vs code | Verify 3002 vs 3000 |

---

## 6. Recommendations

### Immediate Actions

1. **Update version across all documentation** to v0.9.0
2. **Add missing analysis/workspace-memory.md** or remove from index
3. **Document Claude Bridge** — critical new feature
4. **Document Worker command** — `ironclaw worker --help`

### Short-term (Sprint 1-2)

5. Add configuration variables to DEPLOYMENT.md:
   - AGENT_MAX_TOOL_ITERATIONS
   - AGENT_AUTO_APPROVE_TOOLS
   - AGENT_USE_PLANNING
   - AGENT_STUCK_THRESHOLD_SECS
   - SESSION_IDLE_TIMEOUT_SECS

6. Update ARCHITECTURE.md with accurate file counts
7. Add CLI reference for memory commands

### Long-term (Process Improvement)

8. **Implement doc sync CI/CD**: Automatically check version in Cargo.toml matches docs
9. **Add doc linter**: Validate all referenced files exist
10. **Feature flag documentation**: Document which features require which Cargo features

---

## 7. Appendix: Files Analyzed

### Documentation
- ~/src/work/ironclaw-docs/README.md
- ~/src/work/ironclaw-docs/ARCHITECTURE.md
- ~/src/work/ironclaw-docs/DEPLOYMENT.md
- ~/src/work/ironclaw-docs/DEVELOPER-REFERENCE.md
- ~/src/work/ironclaw-docs/analysis/ (11 files)

### Source Code
- ~/src/work/ironclaw/Cargo.toml
- ~/src/work/ironclaw/src/main.rs
- ~/src/work/ironclaw/src/config/agent.rs
- ~/src/work/ironclaw/src/tools/ (directory structure)
- ~/src/work/ironclaw/docs/ (3 files)

---

## 8. Conclusion

The IronClaw documentation provides a solid architectural overview but suffers from version drift and missing coverage of newer features. The v0.9.0 source code includes significant capabilities (Claude Bridge, Worker mode, enhanced configuration) that are absent from the documentation.

**Overall Assessment**: The documentation is useful for understanding core concepts and initial deployment, but contributors and advanced users will encounter gaps when working with the current codebase. Prioritizing the Critical and Major issues above would significantly improve developer experience.

---

*Report generated by Matrix Agent - 2026-02-21*
