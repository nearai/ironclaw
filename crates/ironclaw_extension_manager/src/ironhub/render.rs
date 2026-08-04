use super::model::{IronHubPhase, IronHubResponse};
use crate::terminal_render::{push_line, terminal_safe};

pub fn render_reborn_ironhub_response(label: &str, response: &IronHubResponse) -> String {
    let mut output = String::new();
    push_line(&mut output, format_args!("IronHub {label}"));
    push_line(
        &mut output,
        format_args!(
            "phase: {}",
            match response.phase {
                IronHubPhase::Discovered => "discovered",
                IronHubPhase::Status => "status",
                IronHubPhase::Installed => "installed",
                IronHubPhase::Updated => "updated",
            }
        ),
    );
    push_line(
        &mut output,
        format_args!("total_entries: {}", response.total_entries),
    );
    push_line(
        &mut output,
        format_args!("returned_entries: {}", response.returned_entries),
    );
    if let Some(catalog_total) = response.catalog_total {
        push_line(&mut output, format_args!("catalog_total: {catalog_total}"));
    }
    push_line(
        &mut output,
        format_args!("truncated: {}", response.truncated),
    );
    if let Some(message) = &response.message {
        push_line(
            &mut output,
            format_args!("message: {}", terminal_safe(message)),
        );
    }
    for entry in &response.entries {
        push_line(
            &mut output,
            format_args!(
                "- {} {} {} [{}] ({})",
                entry.kind.as_str(),
                terminal_safe(&entry.name),
                terminal_safe(&entry.version),
                entry.provenance.as_wire(),
                terminal_safe(&entry.description)
            ),
        );
        if let Some(artifact_digest) = &entry.artifact_digest {
            push_line(
                &mut output,
                format_args!("  artifact_digest: {}", terminal_safe(artifact_digest)),
            );
        }
        if let Some(installation) = &entry.installation {
            push_line(
                &mut output,
                format_args!(
                    "  installed: version={} release_tag={} catalog_origin={} artifact_digest={} active={} update_available={}",
                    terminal_safe(&installation.version),
                    terminal_safe(&installation.release_tag),
                    terminal_safe(&installation.catalog_origin),
                    terminal_safe(&installation.artifact_digest),
                    installation.active,
                    installation
                        .update_available
                        .map(|available| available.to_string())
                        .unwrap_or_else(|| "unknown".to_string())
                ),
            );
            for change in &installation.authority_changes {
                push_line(
                    &mut output,
                    format_args!("  authority_change: {}", terminal_safe(change)),
                );
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ironhub::model::{
        IronHubEntryKind, IronHubEntrySummary, IronHubInstallationSummary, IronHubProvenance,
    };

    #[test]
    fn installed_entry_renders_complete_durable_provenance() {
        let response = IronHubResponse {
            phase: IronHubPhase::Status,
            total_entries: 1,
            returned_entries: 1,
            truncated: false,
            catalog_total: Some(1),
            message: None,
            entries: vec![IronHubEntrySummary {
                kind: IronHubEntryKind::Tool,
                name: "example".to_string(),
                version: "2.0.0".to_string(),
                description: "example tool".to_string(),
                provenance: IronHubProvenance::Official,
                artifact_digest: None,
                installation: Some(IronHubInstallationSummary {
                    version: "1.0.0".to_string(),
                    artifact_digest: "sha256:installed".to_string(),
                    release_tag: "v1.0.0".to_string(),
                    catalog_origin: "https://hub.ironclaw.com".to_string(),
                    active: true,
                    update_available: Some(true),
                    authority_changes: Vec::new(),
                }),
            }],
            lifecycle: None,
        };

        let rendered = render_reborn_ironhub_response("status", &response);

        assert!(rendered.contains("release_tag=v1.0.0"));
        assert!(rendered.contains("catalog_origin=https://hub.ironclaw.com"));
        assert!(rendered.contains("artifact_digest=sha256:installed"));
    }
}
