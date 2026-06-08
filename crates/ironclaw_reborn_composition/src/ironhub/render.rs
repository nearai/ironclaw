use ironclaw_product_workflow::{
    LifecyclePackageKind, LifecyclePhase, LifecycleProductPayload, LifecycleProductResponse,
};

pub fn render_reborn_ironhub_response(label: &str, response: &LifecycleProductResponse) -> String {
    let mut output = String::new();
    push_line(&mut output, format_args!("IronHub {label}"));
    push_line(
        &mut output,
        format_args!("phase: {}", phase_label(response.phase)),
    );
    if let Some(package_ref) = &response.package_ref {
        push_line(
            &mut output,
            format_args!(
                "package: {}/{}",
                package_kind_label(package_ref.kind),
                package_ref.id.as_str()
            ),
        );
    }
    if let Some(message) = &response.message {
        push_line(
            &mut output,
            format_args!("message: {}", terminal_safe(message)),
        );
    }
    match response.payload.as_ref() {
        Some(LifecycleProductPayload::ExtensionSearch { extensions, count }) => {
            push_line(&mut output, format_args!("count: {count}"));
            for extension in extensions {
                push_line(
                    &mut output,
                    format_args!(
                        "- tool {} {} ({})",
                        extension.package_ref.id.as_str(),
                        terminal_safe(&extension.version),
                        terminal_safe(&extension.description)
                    ),
                );
            }
        }
        Some(LifecycleProductPayload::CatalogSearch {
            tools,
            skills,
            count,
        }) => {
            push_line(&mut output, format_args!("count: {count}"));
            for tool in tools {
                push_line(
                    &mut output,
                    format_args!(
                        "- tool {} {} ({})",
                        tool.package_ref.id.as_str(),
                        terminal_safe(&tool.version),
                        terminal_safe(&tool.description)
                    ),
                );
            }
            for skill in skills {
                push_line(
                    &mut output,
                    format_args!(
                        "- skill {} {} ({})",
                        skill.name.as_str(),
                        terminal_safe(&skill.version),
                        terminal_safe(&skill.description)
                    ),
                );
            }
        }
        Some(LifecycleProductPayload::SkillSearch {
            skills,
            count,
            truncated,
            ..
        }) => {
            push_line(&mut output, format_args!("count: {count}"));
            push_line(&mut output, format_args!("truncated: {truncated}"));
            for skill in skills {
                push_line(
                    &mut output,
                    format_args!(
                        "- skill {} {} ({})",
                        skill.name.as_str(),
                        terminal_safe(&skill.version),
                        terminal_safe(&skill.description)
                    ),
                );
            }
        }
        Some(LifecycleProductPayload::ExtensionInstall {
            installed,
            visible_capability_ids,
        }) => {
            push_line(&mut output, format_args!("installed: {installed}"));
            for id in visible_capability_ids {
                push_line(
                    &mut output,
                    format_args!("visible_capability: {}", terminal_safe(id)),
                );
            }
        }
        Some(LifecycleProductPayload::SkillInstall { installed, name }) => {
            push_line(&mut output, format_args!("installed: {installed}"));
            push_line(&mut output, format_args!("skill: {}", name.as_str()));
        }
        _ => {}
    }
    output
}

fn phase_label(phase: LifecyclePhase) -> &'static str {
    match phase {
        LifecyclePhase::Discovered => "discovered",
        LifecyclePhase::Installing => "installing",
        LifecyclePhase::Installed => "installed",
        LifecyclePhase::Configured => "configured",
        LifecyclePhase::Activating => "activating",
        LifecyclePhase::Active => "active",
        LifecyclePhase::Disabled => "disabled",
        LifecyclePhase::UpgradeRequired => "upgrade_required",
        LifecyclePhase::Failed => "failed",
        LifecyclePhase::Removing => "removing",
        LifecyclePhase::Removed => "removed",
        LifecyclePhase::UnsupportedOrLegacy => "unsupported_or_legacy",
    }
}

fn package_kind_label(kind: LifecyclePackageKind) -> &'static str {
    match kind {
        LifecyclePackageKind::Extension => "extension",
        LifecyclePackageKind::Skill => "skill",
        LifecyclePackageKind::Mcp => "mcp",
        LifecyclePackageKind::Wasm => "wasm",
    }
}

fn terminal_safe(value: &str) -> String {
    value.chars().flat_map(char::escape_default).collect()
}

fn push_line(output: &mut String, args: std::fmt::Arguments<'_>) {
    use std::fmt::Write as _;
    let _ = output.write_fmt(args);
    output.push('\n');
}
