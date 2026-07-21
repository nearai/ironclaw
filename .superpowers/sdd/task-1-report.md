# Task 1: canonical materialized outbound files

## Result

Implemented the canonical `MaterializedFile<P>` carrier in `ironclaw_attachments` and routed outbound workspace-file delivery through `WorkspaceFile`. `ProjectFsFile` remains a compatibility alias for browse/read responses, while final-reply delivery now requires a project-filesystem reader at construction.

## TDD evidence

The red phase was recorded before implementation:

- `cargo test -p ironclaw_architecture attachment -- --nocapture` failed because `ProductOutboundAttachment` still existed.
- `cargo test -p ironclaw_channel_delivery workspace_attachment -- --nocapture` failed because `WorkspaceFile` was absent and channel delivery still used optional readers.

## Changes

- Added `MaterializedFile<P>`, `WorkspaceFile`, and `ProjectFsFile` in `ironclaw_attachments`.
- Removed the product-adapter mirror carrier and changed the outbound trait to accept `Vec<WorkspaceFile>`.
- Changed project-file reads to return `WorkspaceFile`; browse APIs retain `ProjectFsFile` compatibility.
- Made `FinalReplyDeliveryServices.project_filesystem_reader` mandatory and removed channel-delivery reader builders/options.
- Passed resolver results directly to product adapters after attachment-budget checks.
- Updated Slack, Telegram, WebUI, composition, and test fixtures for the canonical carrier.
- Added an architecture ratchet against a new `ProductOutboundAttachment` or channel-delivery reader builder.

## Validation

Passed:

```text
cargo fmt --all
git diff --check
cargo test -p ironclaw_attachments
cargo test -p ironclaw_channel_delivery workspace_attachment -- --nocapture
cargo test -p ironclaw_product_adapters --features test-support outbound_workspace_file -- --nocapture
cargo test -p ironclaw_architecture attachment -- --nocapture
```

`cargo test -p ironclaw_product_adapters` without `test-support` remains blocked by pre-existing test imports that are gated behind that feature (`FakeOutboundDeliverySink` and related test helpers). The focused adapter contract passes when the crate's test-support feature is enabled.

## Follow-up

The parent controller is responsible for the requested broader product-workflow and composition validation matrix and for reviewing the concurrent attachment-review changes left unstaged by this task.
