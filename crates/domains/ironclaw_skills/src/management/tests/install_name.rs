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
    // A description is required, and is derived from the document's own first line of prose --
    // `name:` alone is the shape discovery silently skips (nearai/ironclaw#7168).
    assert!(written.starts_with("---\nname: daily-digest-email-docs\ndescription: "));
    assert!(written.contains("Summarize updates for an email."));

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
