//! `MemoryPromptContextService` test support (memory-recall envelope seam).

/// Build the `MemoryPromptContextService` the Reborn integration harness wires
/// into the group's single planned runtime (mirrors the production match in
/// `runtime.rs`'s `build_reborn_runtime`). When `filesystem` is `Some`, memory
/// recall reads through the same raw local-dev filesystem `builtin.memory_write`
/// writes into, so a seeded document is discoverable via prompt-context
/// injection too; `None` falls back to `EmptyMemoryPromptContextService`.
#[cfg(feature = "test-support")]
pub fn build_memory_context_source_for_test(
    filesystem: Option<std::sync::Arc<dyn ironclaw_filesystem::RootFilesystem>>,
) -> std::sync::Arc<dyn ironclaw_turns::run_profile::MemoryPromptContextService> {
    match filesystem {
        Some(fs) => std::sync::Arc::new(
            ironclaw_host_runtime::memory_context::ProductionMemoryPromptContextService::new(
                std::sync::Arc::new(
                    ironclaw_memory_native::NativeMemoryService::from_filesystem(fs, None),
                ) as std::sync::Arc<dyn ironclaw_memory::MemoryService>,
            ),
        ),
        None => std::sync::Arc::new(ironclaw_turns::run_profile::EmptyMemoryPromptContextService),
    }
}
