use std::sync::Arc;

use ironclaw_host_api::{CapabilityId, MountView};
use ironclaw_host_runtime::{
    APPLY_PATCH_CAPABILITY_ID, GLOB_CAPABILITY_ID, GREP_CAPABILITY_ID, LIST_DIR_CAPABILITY_ID,
    READ_FILE_CAPABILITY_ID, SHELL_CAPABILITY_ID, WRITE_FILE_CAPABILITY_ID,
};
use ironclaw_turns::run_profile::{
    AgentLoopHostError, CapabilityBatchInvocation, CapabilityBatchOutcome, CapabilityCallCandidate,
    CapabilityDescriptorView, CapabilityInvocation, CapabilityOutcome, LoopCapabilityPort,
    ProviderToolCall, ProviderToolCallCapabilityIds, ProviderToolDefinition,
    VisibleCapabilityRequest, VisibleCapabilitySurface,
};

pub(super) fn wrap_local_dev_surface_disclosure(
    inner: Arc<dyn LoopCapabilityPort>,
    workspace_mounts: &MountView,
) -> Arc<dyn LoopCapabilityPort> {
    let disclosure = LocalDevSurfaceDisclosure::from_workspace_mounts(workspace_mounts);
    if !disclosure.enabled() {
        return inner;
    }
    Arc::new(LocalDevSurfaceDisclosurePort { inner, disclosure })
}

struct LocalDevSurfaceDisclosurePort {
    inner: Arc<dyn LoopCapabilityPort>,
    disclosure: LocalDevSurfaceDisclosure,
}

#[async_trait::async_trait]
impl LoopCapabilityPort for LocalDevSurfaceDisclosurePort {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        let mut definitions = self.inner.tool_definitions()?;
        for definition in &mut definitions {
            self.disclosure.apply_to_tool_definition(definition);
        }
        Ok(definitions)
    }

    fn provider_tool_call_capability_ids(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<ProviderToolCallCapabilityIds, AgentLoopHostError> {
        self.inner.provider_tool_call_capability_ids(tool_call)
    }

    fn validate_provider_tool_call(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<(), AgentLoopHostError> {
        self.inner.validate_provider_tool_call(tool_call)
    }

    async fn register_provider_tool_call(
        &self,
        tool_call: ProviderToolCall,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        self.inner.register_provider_tool_call(tool_call).await
    }

    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        let mut surface = self.inner.visible_capabilities(request).await?;
        for descriptor in &mut surface.descriptors {
            self.disclosure.apply_to_descriptor(descriptor);
        }
        Ok(surface)
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        self.inner.invoke_capability(request).await
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        self.inner.invoke_capability_batch(request).await
    }
}

struct LocalDevSurfaceDisclosure {
    scoped_roots_note: Option<String>,
}

impl LocalDevSurfaceDisclosure {
    fn from_workspace_mounts(workspace_mounts: &MountView) -> Self {
        let aliases = model_visible_workspace_aliases(workspace_mounts);
        Self {
            scoped_roots_note: confirmed_host_roots_note(&aliases),
        }
    }

    fn enabled(&self) -> bool {
        self.scoped_roots_note.is_some()
    }

    fn apply_to_descriptor(&self, descriptor: &mut CapabilityDescriptorView) {
        self.apply_to_surface_fields(
            &descriptor.capability_id,
            &mut descriptor.safe_description,
            &mut descriptor.parameters_schema,
        );
    }

    fn apply_to_tool_definition(&self, definition: &mut ProviderToolDefinition) {
        self.apply_to_surface_fields(
            &definition.capability_id,
            &mut definition.description,
            &mut definition.parameters,
        );
    }

    fn apply_to_surface_fields(
        &self,
        capability_id: &CapabilityId,
        description: &mut String,
        parameters_schema: &mut serde_json::Value,
    ) {
        if capability_id.as_str() == SHELL_CAPABILITY_ID {
            append_description_note(description, LOCAL_DEV_LOCAL_HOST_SHELL_NOTE);
            return;
        }
        if !local_dev_scoped_path_capability(capability_id.as_str()) {
            return;
        }
        let Some(note) = self.scoped_roots_note.as_deref() else {
            return;
        };
        append_description_note(description, note);
        append_path_schema_note(parameters_schema, note);
    }
}

const LOCAL_DEV_LOCAL_HOST_SHELL_NOTE: &str = "Runs on the local host with local-dev shell process and network access. Local-host shell command paths may use /workspace and /host aliases when those roots are configured; they are translated to the confirmed local workspace and host-home paths before execution.";

fn local_dev_scoped_path_capability(capability_id: &str) -> bool {
    matches!(
        capability_id,
        READ_FILE_CAPABILITY_ID
            | WRITE_FILE_CAPABILITY_ID
            | LIST_DIR_CAPABILITY_ID
            | GLOB_CAPABILITY_ID
            | GREP_CAPABILITY_ID
            | APPLY_PATCH_CAPABILITY_ID
    )
}

fn model_visible_workspace_aliases(workspace_mounts: &MountView) -> Vec<&str> {
    let mut aliases = Vec::new();
    for mount in &workspace_mounts.mounts {
        let alias = mount.alias.as_str();
        if matches!(alias, "/workspace" | "/host") && !aliases.contains(&alias) {
            aliases.push(alias);
        }
    }
    aliases
}

fn confirmed_host_roots_note(aliases: &[&str]) -> Option<String> {
    if !aliases.contains(&"/host") {
        return None;
    }
    let roots = aliases.join(", ");
    let mut note = format!("Available scoped roots: {roots}.");
    note.push_str(" /host is the confirmed host home mount; prefer /host over raw home paths.");
    note.push_str(" In local-host shell, the same roots are available as command path aliases.");
    Some(note)
}

fn append_description_note(description: &mut String, note: &str) {
    if description.contains(note) {
        return;
    }
    if !description.ends_with('.') {
        description.push('.');
    }
    description.push(' ');
    description.push_str(note);
}

fn append_path_schema_note(schema: &mut serde_json::Value, note: &str) {
    let Some(path) = schema
        .get_mut("properties")
        .and_then(serde_json::Value::as_object_mut)
        .and_then(|properties| properties.get_mut("path"))
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    let Some(description) = path
        .get("description")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
    else {
        return;
    };
    if description.contains(note) {
        return;
    }
    path.insert(
        "description".to_string(),
        serde_json::Value::String(format!("{description} {note}")),
    );
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use ironclaw_host_api::MountPermissions;

    use super::*;

    #[test]
    fn disclosure_is_disabled_without_confirmed_host_mount() {
        let workspace_mounts =
            crate::local_dev_mounts::workspace_mount_view(MountPermissions::read_write(), &[])
                .expect("workspace mounts build");

        let disclosure = LocalDevSurfaceDisclosure::from_workspace_mounts(&workspace_mounts);

        assert!(disclosure.scoped_roots_note.is_none());
    }

    #[test]
    fn disclosure_redacts_raw_host_home_aliases() {
        let workspace_mounts = crate::local_dev_mounts::workspace_mount_view(
            MountPermissions::read_write(),
            &[Path::new("/Users/alice")],
        )
        .expect("workspace mounts build");

        let disclosure = LocalDevSurfaceDisclosure::from_workspace_mounts(&workspace_mounts);
        let note = disclosure
            .scoped_roots_note
            .expect("confirmed host mount is disclosed");

        assert!(note.contains("/workspace, /host"));
        assert!(!note.contains("/Users/alice"));
    }
}
