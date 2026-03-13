# Builder Sub-Tool Approval Context Fix - Implementation Summary

## Overview

Fixed the issue where `build_software` tool bypasses tool approval checks when executing sub-tools (shell, write_file, etc.). This prevented the builder from working in autonomous contexts (web UI, routines).

## Root Cause

The `build_software` tool called `tool.execute()` directly with a dummy `JobContext::default()` that had no approval context. This bypassed the normal worker-level approval checks.

## Changes Made

### 1. src/context/state.rs
- Added `approval_context: Option<ApprovalContext>` field to `JobContext` struct
- Added `with_approval_context()` method to set the approval context
- Updated `Default` impl to include `ApprovalContext::autonomous()`
- Added import: `use crate::tools::tool::ApprovalContext;`

### 2. src/tools/tool.rs
- Added `check_approval_in_context()` helper function
- Allows tools to verify approval before executing sub-tools
- Returns `Err(ToolError::NotAuthorized)` if blocked

### 3. src/tools/builder/core.rs
- Added import for `ApprovalContext` and `check_approval_in_context`
- Updated `execute_build_tool()` to:
  - Create context with build-specific approval permissions
  - Call `check_approval_in_context()` before executing sub-tools
  - Explicitly allows: shell, read_file, write_file, list_dir, apply_patch, http

### 4. src/worker/job.rs
- Moved `job_ctx` fetch before approval check
- Added job-level approval context check (takes precedence over worker-level)
- Falls back to worker-level `deps.approval_context` if job-level is None

### 5. src/agent/scheduler.rs
- Updated `dispatch_job_inner()` to store `approval_context` in JobContext
- Ensures approval context propagates from scheduler to worker to tools

### 6. tests/tool_approval_context.rs
- Added comprehensive tests for approval context functionality
- Tests cover: default context, with_approval_context, autonomous blocking, builder tools

## Verification Steps

1. **Build from source**:
   ```bash
   cd ~/Code/ironclaw
   cargo build --release
   cargo install --path .
   ```

2. **Test builder in web UI**:
   - Try `build_software` - should no longer get "authentication required"
   - Builder should be able to execute shell, write_file, etc.

3. **Run tests**:
   ```bash
   cargo test tool_approval_context
   ```

4. **Regression testing**:
   - Verify existing approval flows still work (chat sessions, routines)
   - Verify tools respect approval in different contexts

## Related Issues/PRs

- References #922 (relaxed approval requirements for build_software)
- #577 (approval context for autonomous job execution)
- #257 (enable tool access in lightweight routine execution)

## Architecture Notes

The fix introduces a two-level approval system:
1. **Job-level** (`job_ctx.approval_context`): More specific, set by tools like builder
2. **Worker-level** (`deps.approval_context`): Fallback, set by scheduler for autonomous jobs

When a tool needs to execute sub-tools, it creates a JobContext with the appropriate approval context and calls `check_approval_in_context()` to verify permissions before execution.
