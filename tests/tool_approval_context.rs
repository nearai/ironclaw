//! Tests for tool approval context propagation.
//!
//! Verifies that:
//! - JobContext carries approval_context through the execution chain
//! - Builder sub-tools use proper approval checks
//! - Worker checks job-level approval context

use ironclaw::context::JobContext;
use ironclaw::tools::{
    ApprovalContext, ApprovalRequirement, Tool, ToolError, ToolOutput, check_approval_in_context,
};

/// A simple test tool that requires approval.
#[derive(Debug)]
struct TestTool {
    approval_req: ApprovalRequirement,
}

#[async_trait::async_trait]
impl Tool for TestTool {
    fn name(&self) -> &str {
        "test_tool"
    }

    fn description(&self) -> &str {
        "A test tool for approval checking"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
        })
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput::text("ok", std::time::Duration::from_millis(1)))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        self.approval_req
    }
}

#[test]
fn test_job_context_default_has_no_approval_context() {
    let ctx = JobContext::default();
    assert!(
        ctx.approval_context.is_none(),
        "JobContext::default() should NOT have approval_context set for security"
    );
}

#[test]
fn test_job_context_with_approval_context() {
    let ctx =
        JobContext::new("Test", "Test job").with_approval_context(ApprovalContext::autonomous());
    assert!(ctx.approval_context.is_some());
}

#[test]
fn test_approval_context_autonomous_blocks_unless_auto_approved_without_allowlist() {
    let ctx =
        JobContext::new("Test", "Test job").with_approval_context(ApprovalContext::autonomous());
    let tool = TestTool {
        approval_req: ApprovalRequirement::UnlessAutoApproved,
    };

    let result = check_approval_in_context(
        &ctx,
        "test_tool",
        tool.requires_approval(&serde_json::json!({})),
    );
    assert!(
        result.is_err(),
        "UnlessAutoApproved should be blocked unless the tool is in the autonomous allowlist"
    );
}

#[test]
fn test_approval_context_autonomous_blocks_always() {
    let ctx =
        JobContext::new("Test", "Test job").with_approval_context(ApprovalContext::autonomous());
    let tool = TestTool {
        approval_req: ApprovalRequirement::Always,
    };

    // Check should fail for Always in autonomous context without explicit allow
    let result = check_approval_in_context(
        &ctx,
        "test_tool",
        tool.requires_approval(&serde_json::json!({})),
    );
    assert!(
        result.is_err(),
        "Always should be blocked in autonomous context"
    );
    assert!(matches!(result, Err(ToolError::NotAuthorized(_))));
}

#[test]
fn test_approval_context_autonomous_with_tools_allows_specific() {
    let ctx = JobContext::new("Test", "Test job").with_approval_context(
        ApprovalContext::autonomous_with_tools(["shell".to_string(), "read_file".to_string()]),
    );
    let always_tool = TestTool {
        approval_req: ApprovalRequirement::Always,
    };
    let soft_tool = TestTool {
        approval_req: ApprovalRequirement::UnlessAutoApproved,
    };

    // shell should be allowed (explicitly listed)
    check_approval_in_context(
        &ctx,
        "shell",
        always_tool.requires_approval(&serde_json::json!({})),
    )
    .expect("Listed tool should be allowed");

    // read_file should be allowed (explicitly listed)
    check_approval_in_context(
        &ctx,
        "read_file",
        always_tool.requires_approval(&serde_json::json!({})),
    )
    .expect("Listed tool should be allowed");

    // write_file should be blocked (not listed)
    let result = check_approval_in_context(
        &ctx,
        "write_file",
        always_tool.requires_approval(&serde_json::json!({})),
    );
    assert!(result.is_err(), "Non-listed Always tool should be blocked");

    // Unlisted UnlessAutoApproved tools should also be blocked.
    let result = check_approval_in_context(
        &ctx,
        "http",
        soft_tool.requires_approval(&serde_json::json!({})),
    );
    assert!(
        result.is_err(),
        "Non-listed UnlessAutoApproved tool should be blocked"
    );

    // But listed UnlessAutoApproved tools remain allowed.
    check_approval_in_context(
        &ctx,
        "shell",
        soft_tool.requires_approval(&serde_json::json!({})),
    )
    .expect("Listed UnlessAutoApproved tool should be allowed");
}

#[test]
fn test_builder_tools_approval_context() {
    // Verify the builder creates the correct approval context
    let ctx = JobContext::new("Test", "Test job").with_approval_context(
        ApprovalContext::autonomous_with_tools([
            "shell".into(),
            "read_file".into(),
            "write_file".into(),
            "list_dir".into(),
            "apply_patch".into(),
        ]),
    );

    let tool = TestTool {
        approval_req: ApprovalRequirement::Always,
    };

    // All build tools should be allowed
    for tool_name in &[
        "shell",
        "read_file",
        "write_file",
        "list_dir",
        "apply_patch",
    ] {
        check_approval_in_context(
            &ctx,
            tool_name,
            tool.requires_approval(&serde_json::json!({})),
        )
        .unwrap_or_else(|e| panic!("Builder tool '{}' should be allowed, got: {}", tool_name, e));
    }

    // Non-build tools should be blocked
    let result = check_approval_in_context(
        &ctx,
        "create_job",
        tool.requires_approval(&serde_json::json!({})),
    );
    assert!(result.is_err(), "Non-build Always tool should be blocked");
}

#[test]
fn test_default_context_blocks_non_never_tools() {
    // JobContext::default() has no approval_context, which should block
    // all non-Never tools (secure default)
    let ctx = JobContext::default();

    let tool = TestTool {
        approval_req: ApprovalRequirement::UnlessAutoApproved,
    };

    // UnlessAutoApproved should be blocked with no approval_context
    let result = check_approval_in_context(
        &ctx,
        "test_tool",
        tool.requires_approval(&serde_json::json!({})),
    );
    assert!(
        result.is_err(),
        "UnlessAutoApproved should be blocked with no approval_context"
    );

    let always_tool = TestTool {
        approval_req: ApprovalRequirement::Always,
    };
    let result = check_approval_in_context(
        &ctx,
        "test_tool",
        always_tool.requires_approval(&serde_json::json!({})),
    );
    assert!(
        result.is_err(),
        "Always should be blocked with no approval_context"
    );

    // Never should still be allowed
    let never_tool = TestTool {
        approval_req: ApprovalRequirement::Never,
    };
    check_approval_in_context(
        &ctx,
        "test_tool",
        never_tool.requires_approval(&serde_json::json!({})),
    )
    .expect("Never should be allowed even with no approval_context");
}

#[test]
fn test_never_tools_allowed_in_additive_model() {
    // Never tools (echo, time, etc.) should always be allowed regardless of
    // approval context configuration - they don't need to be in the allowlist.
    let ctx = JobContext::new("Test", "Test job").with_approval_context(
        ApprovalContext::autonomous_with_tools(["shell".to_string()]),
    );

    // A Never tool not in the allowlist should still be allowed
    check_approval_in_context(&ctx, "echo", ApprovalRequirement::Never)
        .expect("Never tools should pass without being in the allowlist");

    // And of course a Never tool in the allowlist should also be allowed
    check_approval_in_context(&ctx, "shell", ApprovalRequirement::Never)
        .expect("Never tools in the allowlist should also pass");

    // But an Always tool not in the allowlist should be blocked
    let result = check_approval_in_context(&ctx, "echo", ApprovalRequirement::Always);
    assert!(
        result.is_err(),
        "Always tools NOT in allowlist should be blocked"
    );
}

#[test]
fn test_builder_execute_build_tool_blocks_unlisted_tool() {
    // The builder creates a context with specific allowed tools.
    // Tools NOT in the builder's allowlist should be blocked.
    let builder_ctx = JobContext::new("Build", "Building software").with_approval_context(
        ApprovalContext::autonomous_with_tools([
            "shell".into(),
            "read_file".into(),
            "write_file".into(),
            "list_dir".into(),
            "apply_patch".into(),
        ]),
    );

    // Builder-allowed tools should pass
    for tool_name in &[
        "shell",
        "read_file",
        "write_file",
        "list_dir",
        "apply_patch",
    ] {
        check_approval_in_context(&builder_ctx, tool_name, ApprovalRequirement::Always)
            .unwrap_or_else(|e| {
                panic!("Builder tool '{}' should be allowed, got: {}", tool_name, e)
            });
    }

    // Tools NOT in builder's allowlist should be blocked (e.g., http, create_job, message)
    for tool_name in &["http", "create_job", "message", "secret_save"] {
        let result =
            check_approval_in_context(&builder_ctx, tool_name, ApprovalRequirement::Always);
        assert!(
            result.is_err(),
            "Tool '{}' should be blocked by builder context (not in allowlist)",
            tool_name
        );
    }

    let result = check_approval_in_context(
        &builder_ctx,
        "http",
        ApprovalRequirement::UnlessAutoApproved,
    );
    assert!(
        result.is_err(),
        "UnlessAutoApproved tools outside the builder allowlist should also be blocked"
    );
}
