use std::sync::Arc;

use ironclaw_filesystem::{InMemoryBackend, RootFilesystem};
use ironclaw_host_api::{
    MountAlias, MountGrant, MountPermissions, MountView, ResourceScope, VirtualPath,
};

use super::*;

#[tokio::test]
async fn install_normalizes_plain_markdown_requested_display_name() {
    let filesystem = Arc::new(InMemoryBackend::default());
    let context = skill_management_context(filesystem.clone(), skill_mounts());

    let installed = install_skill(
        &context,
        SkillInstallRequest {
            name: Some("Daily Digest Email Docs"),
            content: "# Daily Digest\n\nSummarize updates for an email.\n",
            files: &[],
            source: SkillInstallSource::User,
            source_url: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(installed.name, "daily-digest-email-docs");
    let written = read_file(
        filesystem.as_ref(),
        "/projects/skills/daily-digest-email-docs/SKILL.md",
    )
    .await;
    assert!(written.starts_with("---\nname: daily-digest-email-docs\n---\n\n"));

    let listed = list_skills(&context).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "daily-digest-email-docs");
}

#[tokio::test]
async fn install_accepts_frontmatter_matching_requested_display_name() {
    let filesystem = Arc::new(InMemoryBackend::default());
    let context = skill_management_context(filesystem, skill_mounts());

    let installed = install_skill(
        &context,
        SkillInstallRequest {
            name: Some("Daily Digest Email Docs"),
            content: &skill_md(
                "daily-digest-email-docs",
                "daily digest description",
                "DAILY_DIGEST_PROMPT",
            ),
            files: &[],
            source: SkillInstallSource::User,
            source_url: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(installed.name, "daily-digest-email-docs");
    let listed = list_skills(&context).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "daily-digest-email-docs");
}

fn skill_mounts() -> MountView {
    MountView::new(vec![
        MountGrant::new(
            MountAlias::new("/skills").unwrap(),
            VirtualPath::new("/projects/skills").unwrap(),
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/system/skills").unwrap(),
            VirtualPath::new("/projects/system/skills").unwrap(),
            MountPermissions::read_only(),
        ),
    ])
    .unwrap()
}

fn skill_management_context(
    filesystem: Arc<InMemoryBackend>,
    mounts: MountView,
) -> SkillManagementContext {
    let filesystem: Arc<dyn RootFilesystem> = filesystem;
    SkillManagementContext::new(filesystem, mounts, ResourceScope::system())
}

fn skill_md(name: &str, description: &str, prompt: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---\n{prompt}\n")
}

async fn read_file(root: &InMemoryBackend, path: &str) -> String {
    let bytes = root
        .read_file_bounded(&VirtualPath::new(path).unwrap(), 1024)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("{path} should exist"));
    String::from_utf8(bytes).unwrap()
}
