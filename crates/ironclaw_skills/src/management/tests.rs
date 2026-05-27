use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{
    BackendCapabilities, DirEntry, FileStat, FilesystemError, FilesystemOperation, InMemoryBackend,
    RootFilesystem,
};
use ironclaw_host_api::{
    MountAlias, MountGrant, MountPermissions, MountView, ResourceScope, ScopedPath, VirtualPath,
};

use super::install_bundle::MAX_INSTALL_BUNDLE_FILE_BYTES;
use super::*;

#[tokio::test]
async fn install_list_and_remove_user_skills_through_scoped_mounts() {
    let filesystem = Arc::new(InMemoryBackend::default());
    write_file(
        filesystem.as_ref(),
        "/projects/system/skills/system-helper/SKILL.md",
        skill_md(
            "system-helper",
            "system skill description",
            "SYSTEM_SKILL_PROMPT",
        ),
    )
    .await;
    let context = skill_management_context(filesystem.clone(), skill_mounts());

    let installed = install_skill(
        &context,
        SkillInstallRequest {
            name: None,
            content: &skill_md(
                "local-helper",
                "local skill description",
                "LOCAL_SKILL_PROMPT",
            ),
            files: &[],
            source: SkillInstallSource::User,
            source_url: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(installed.name, "local-helper");
    assert_eq!(
        installed.scoped_path,
        "/skills/local-helper/SKILL.md".to_string()
    );

    let listed = list_skills(&context).await.unwrap();
    assert_eq!(listed.len(), 2);
    assert!(
        listed
            .iter()
            .any(|skill| skill.name == "system-helper" && skill.source == SkillSource::System)
    );
    assert!(
        listed
            .iter()
            .any(|skill| skill.name == "local-helper" && skill.source == SkillSource::User)
    );

    let removed = remove_skill(
        &context,
        SkillRemoveRequest {
            name: "local-helper",
        },
    )
    .await
    .unwrap();
    assert_eq!(removed.name, "local-helper");
    assert_eq!(list_skills(&context).await.unwrap().len(), 1);
}

#[tokio::test]
async fn install_rejects_name_mismatch() {
    let filesystem = Arc::new(InMemoryBackend::default());
    let context = skill_management_context(filesystem, skill_mounts());

    let error = install_skill(
        &context,
        SkillInstallRequest {
            name: Some("expected"),
            content: &skill_md("actual", "description", "PROMPT"),
            files: &[],
            source: SkillInstallSource::User,
            source_url: None,
        },
    )
    .await
    .unwrap_err();

    assert_eq!(error.kind(), SkillManagementErrorKind::InvalidInput);
}

#[tokio::test]
async fn install_accepts_named_plain_markdown_content() {
    let filesystem = Arc::new(InMemoryBackend::default());
    let context = skill_management_context(filesystem.clone(), skill_mounts());

    let installed = install_skill(
        &context,
        SkillInstallRequest {
            name: Some("qa-smoke-skill"),
            content: "# QA Smoke\n\nSay \"qa skill loaded\" when asked.\n",
            files: &[],
            source: SkillInstallSource::User,
            source_url: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(installed.name, "qa-smoke-skill");
    let written = read_file(
        filesystem.as_ref(),
        "/projects/skills/qa-smoke-skill/SKILL.md",
    )
    .await;
    assert!(written.starts_with("---\nname: qa-smoke-skill\n---\n\n"));
    assert!(written.contains("Say \"qa skill loaded\""));

    let listed = list_skills(&context).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "qa-smoke-skill");
}

#[tokio::test]
async fn install_rejects_malformed_frontmatter_even_with_requested_name() {
    let filesystem = Arc::new(InMemoryBackend::default());
    let context = skill_management_context(filesystem, skill_mounts());

    let error = install_skill(
        &context,
        SkillInstallRequest {
            name: Some("qa-smoke-skill"),
            content: "---\nname: qa-smoke-skill\n\nMissing closing delimiter.\n",
            files: &[],
            source: SkillInstallSource::User,
            source_url: None,
        },
    )
    .await
    .unwrap_err();

    assert_eq!(error.kind(), SkillManagementErrorKind::InvalidInput);
    assert!(
        error
            .reason()
            .is_some_and(|reason| reason.contains("Missing YAML frontmatter")),
        "parse context should be preserved in the public error reason: {error:?}"
    );
}

#[tokio::test]
async fn install_rejects_plain_markdown_when_synthesized_content_exceeds_prompt_limit() {
    let filesystem = Arc::new(InMemoryBackend::default());
    let context = skill_management_context(filesystem.clone(), skill_mounts());
    let header = "---\nname: qa-smoke-skill\n---\n\n";
    let content = "x".repeat(MAX_PROMPT_FILE_SIZE as usize - header.len() + 1);

    let error = install_skill(
        &context,
        SkillInstallRequest {
            name: Some("qa-smoke-skill"),
            content: &content,
            files: &[],
            source: SkillInstallSource::User,
            source_url: None,
        },
    )
    .await
    .unwrap_err();

    assert_eq!(error.kind(), SkillManagementErrorKind::Resource);
    assert_missing(
        filesystem.as_ref(),
        "/projects/skills/qa-smoke-skill/SKILL.md",
    )
    .await;
}

#[tokio::test]
async fn install_preserves_parse_error_context() {
    let filesystem = Arc::new(InMemoryBackend::default());
    let context = skill_management_context(filesystem, skill_mounts());

    let error = install_skill(
        &context,
        SkillInstallRequest {
            name: None,
            content: "not a skill manifest",
            files: &[],
            source: SkillInstallSource::User,
            source_url: None,
        },
    )
    .await
    .unwrap_err();

    assert_eq!(error.kind(), SkillManagementErrorKind::InvalidInput);
    assert!(
        error
            .reason()
            .is_some_and(|reason| reason.contains("Missing YAML frontmatter")),
        "parse context should be preserved in the public error reason: {error:?}"
    );
}

#[tokio::test]
async fn install_rejects_invalid_bundle_files() {
    let cases = [
        (
            "../escape.md",
            b"ok".as_slice(),
            SkillManagementErrorKind::InvalidInput,
        ),
        (
            "/absolute.md",
            b"ok".as_slice(),
            SkillManagementErrorKind::InvalidInput,
        ),
        (
            "SKILL.md",
            b"ok".as_slice(),
            SkillManagementErrorKind::InvalidInput,
        ),
        (
            ".ironclaw-install.json",
            b"ok".as_slice(),
            SkillManagementErrorKind::InvalidInput,
        ),
    ];

    for (relative_path, contents, expected) in cases {
        let filesystem = Arc::new(InMemoryBackend::default());
        let context = skill_management_context(filesystem, skill_mounts());

        let error = install_skill(
            &context,
            SkillInstallRequest {
                name: None,
                content: &skill_md("bundle-helper", "description", "PROMPT"),
                files: &[SkillInstallFile {
                    relative_path,
                    contents,
                }],
                source: SkillInstallSource::User,
                source_url: None,
            },
        )
        .await
        .unwrap_err();

        assert_eq!(error.kind(), expected);
    }

    let oversized = vec![b'x'; MAX_INSTALL_BUNDLE_FILE_BYTES + 1];
    let filesystem = Arc::new(InMemoryBackend::default());
    let context = skill_management_context(filesystem, skill_mounts());
    let error = install_skill(
        &context,
        SkillInstallRequest {
            name: None,
            content: &skill_md("oversized-helper", "description", "PROMPT"),
            files: &[SkillInstallFile {
                relative_path: "references/large.bin",
                contents: &oversized,
            }],
            source: SkillInstallSource::User,
            source_url: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind(), SkillManagementErrorKind::Resource);

    let paths = (0..=MAX_INSTALL_BUNDLE_FILES)
        .map(|index| format!("references/{index}.md"))
        .collect::<Vec<_>>();
    let files = paths
        .iter()
        .map(|path| SkillInstallFile {
            relative_path: path.as_str(),
            contents: b"ok",
        })
        .collect::<Vec<_>>();
    let filesystem = Arc::new(InMemoryBackend::default());
    let context = skill_management_context(filesystem, skill_mounts());
    let error = install_skill(
        &context,
        SkillInstallRequest {
            name: None,
            content: &skill_md("too-many-helper", "description", "PROMPT"),
            files: &files,
            source: SkillInstallSource::User,
            source_url: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind(), SkillManagementErrorKind::Resource);
}

#[tokio::test]
async fn install_bundle_failure_cleans_up_partial_directory() {
    let inner = Arc::new(InMemoryBackend::default());
    let filesystem = Arc::new(FailingBundleWriteFilesystem {
        inner: inner.clone(),
        fail_suffix: "/scripts/run.py",
        fail_delete: false,
    });
    let context = skill_management_context_with_root(filesystem, skill_mounts());

    let error = install_skill(
        &context,
        SkillInstallRequest {
            name: None,
            content: &skill_md("partial-helper", "description", "PROMPT"),
            files: &[
                SkillInstallFile {
                    relative_path: "references/guide.md",
                    contents: b"# Guide\n",
                },
                SkillInstallFile {
                    relative_path: "scripts/run.py",
                    contents: b"print('nope')\n",
                },
            ],
            source: SkillInstallSource::User,
            source_url: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind(), SkillManagementErrorKind::FilesystemDenied);

    assert_missing(&inner, "/projects/skills/partial-helper/SKILL.md").await;
    assert_missing(
        &inner,
        "/projects/skills/partial-helper/references/guide.md",
    )
    .await;
}

#[tokio::test]
async fn install_rejects_preexisting_skill_directory_without_deleting_contents() {
    let filesystem = Arc::new(InMemoryBackend::default());
    filesystem
        .write_file(
            &VirtualPath::new("/projects/skills/existing-helper/references/guide.md").unwrap(),
            b"# Keep\n",
        )
        .await
        .unwrap();
    let context = skill_management_context(filesystem.clone(), skill_mounts());

    let error = install_skill(
        &context,
        SkillInstallRequest {
            name: None,
            content: &skill_md("existing-helper", "description", "PROMPT"),
            files: &[SkillInstallFile {
                relative_path: "scripts/run.py",
                contents: b"print('new')\n",
            }],
            source: SkillInstallSource::User,
            source_url: None,
        },
    )
    .await
    .unwrap_err();

    assert_eq!(error.kind(), SkillManagementErrorKind::Conflict);
    assert_file_contents(
        &filesystem,
        "/projects/skills/existing-helper/references/guide.md",
        b"# Keep\n",
    )
    .await;
    assert_missing(&filesystem, "/projects/skills/existing-helper/SKILL.md").await;
    assert_missing(
        &filesystem,
        "/projects/skills/existing-helper/scripts/run.py",
    )
    .await;
}

#[tokio::test]
async fn install_serializes_concurrent_same_name_requests() {
    let filesystem = Arc::new(InMemoryBackend::default());
    let context = skill_management_context(filesystem.clone(), skill_mounts());
    let content = skill_md("shared-helper", "description", "PROMPT");

    let first = install_skill(
        &context,
        SkillInstallRequest {
            name: None,
            content: &content,
            files: &[],
            source: SkillInstallSource::User,
            source_url: None,
        },
    );
    let second = install_skill(
        &context,
        SkillInstallRequest {
            name: None,
            content: &content,
            files: &[],
            source: SkillInstallSource::User,
            source_url: None,
        },
    );
    let (first, second) = tokio::join!(first, second);

    let results = [first, second];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| {
                result
                    .as_ref()
                    .is_err_and(|error| error.kind() == SkillManagementErrorKind::Conflict)
            })
            .count(),
        1
    );
    assert_file_contents(
        &filesystem,
        "/projects/skills/shared-helper/SKILL.md",
        content.as_bytes(),
    )
    .await;
}

#[tokio::test]
async fn install_metadata_write_failure_cleans_up_partial_directory() {
    let inner = Arc::new(InMemoryBackend::default());
    let filesystem = Arc::new(FailingBundleWriteFilesystem {
        inner: inner.clone(),
        fail_suffix: "/.ironclaw-install.json",
        fail_delete: false,
    });
    let context = skill_management_context_with_root(filesystem, skill_mounts());

    let error = install_skill(
        &context,
        SkillInstallRequest {
            name: None,
            content: &skill_md("metadata-helper", "description", "PROMPT"),
            files: &[SkillInstallFile {
                relative_path: "references/guide.md",
                contents: b"# Guide\n",
            }],
            source: SkillInstallSource::InstalledUrl,
            source_url: Some("https://example.test/SKILL.md"),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind(), SkillManagementErrorKind::InvalidSkill);

    assert_missing(&inner, "/projects/skills/metadata-helper/SKILL.md").await;
    assert_missing(
        &inner,
        "/projects/skills/metadata-helper/references/guide.md",
    )
    .await;
}

#[tokio::test]
async fn install_cleanup_failure_is_reported() {
    let inner = Arc::new(InMemoryBackend::default());
    let filesystem = Arc::new(FailingBundleWriteFilesystem {
        inner: inner.clone(),
        fail_suffix: "/scripts/run.py",
        fail_delete: true,
    });
    let context = skill_management_context_with_root(filesystem, skill_mounts());

    let error = install_skill(
        &context,
        SkillInstallRequest {
            name: None,
            content: &skill_md("cleanup-helper", "description", "PROMPT"),
            files: &[
                SkillInstallFile {
                    relative_path: "references/guide.md",
                    contents: b"# Guide\n",
                },
                SkillInstallFile {
                    relative_path: "scripts/run.py",
                    contents: b"print('nope')\n",
                },
            ],
            source: SkillInstallSource::User,
            source_url: None,
        },
    )
    .await
    .unwrap_err();

    assert_eq!(error.kind(), SkillManagementErrorKind::InvalidSkill);
    assert_file_contents(
        &inner,
        "/projects/skills/cleanup-helper/references/guide.md",
        b"# Guide\n",
    )
    .await;
}

#[tokio::test]
async fn list_treats_malformed_install_metadata_as_installed() {
    let filesystem = Arc::new(InMemoryBackend::default());
    write_file(
        filesystem.as_ref(),
        "/projects/skills/metadata-helper/SKILL.md",
        skill_md("metadata-helper", "local skill description", "PROMPT"),
    )
    .await;
    filesystem
        .write_file(
            &VirtualPath::new("/projects/skills/metadata-helper/.ironclaw-install.json").unwrap(),
            b"not json",
        )
        .await
        .unwrap();
    let context = skill_management_context(filesystem, skill_mounts());

    let listed = list_skills(&context).await.unwrap();

    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "metadata-helper");
    assert_eq!(listed[0].source, SkillSource::Installed);
}

#[tokio::test]
async fn list_treats_oversized_install_metadata_as_installed() {
    let filesystem = Arc::new(InMemoryBackend::default());
    write_file(
        filesystem.as_ref(),
        "/projects/skills/metadata-helper/SKILL.md",
        skill_md("metadata-helper", "local skill description", "PROMPT"),
    )
    .await;
    filesystem
        .write_file(
            &VirtualPath::new("/projects/skills/metadata-helper/.ironclaw-install.json").unwrap(),
            &vec![b'x'; crate::MAX_INSTALL_METADATA_BYTES + 1],
        )
        .await
        .unwrap();
    let context = skill_management_context(filesystem, skill_mounts());

    let listed = list_skills(&context).await.unwrap();

    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "metadata-helper");
    assert_eq!(listed[0].source, SkillSource::Installed);
}

#[tokio::test]
async fn list_treats_unmounted_optional_skill_root_as_empty() {
    let filesystem = Arc::new(InMemoryBackend::default());
    write_file(
        filesystem.as_ref(),
        "/projects/skills/local-helper/SKILL.md",
        skill_md("local-helper", "local skill description", "PROMPT"),
    )
    .await;
    let context = skill_management_context(filesystem, user_skill_mounts());

    let listed = list_skills(&context).await.unwrap();

    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "local-helper");
    assert_eq!(listed[0].source, SkillSource::User);
}

#[tokio::test]
async fn search_skills_empty_query_returns_all_matching_skills() {
    let filesystem = Arc::new(InMemoryBackend::default());
    write_file(
        filesystem.as_ref(),
        "/projects/system/skills/system-helper/SKILL.md",
        skill_md(
            "system-helper",
            "system skill description",
            "SYSTEM_SKILL_PROMPT",
        ),
    )
    .await;
    write_file(
        filesystem.as_ref(),
        "/projects/skills/local-helper/SKILL.md",
        skill_md("local-helper", "local skill description", "LOCAL_PROMPT"),
    )
    .await;
    let context = skill_management_context(filesystem, skill_mounts());

    let result = search_skills(
        &context,
        SkillSearchRequest {
            query: "",
            limit: 10,
        },
    )
    .await
    .unwrap();

    assert_eq!(result.skills.len(), 2);
    assert!(!result.truncated);
    assert!(
        result
            .skills
            .iter()
            .any(|skill| skill.name == "system-helper")
    );
    assert!(
        result
            .skills
            .iter()
            .any(|skill| skill.name == "local-helper")
    );
}

#[tokio::test]
async fn search_skills_returns_bounded_matches_with_truncation() {
    let filesystem = Arc::new(InMemoryBackend::default());
    for name in ["alpha-helper", "beta-helper", "gamma-helper"] {
        write_file(
            filesystem.as_ref(),
            &format!("/projects/skills/{name}/SKILL.md"),
            skill_md(name, "helper description", "PROMPT"),
        )
        .await;
    }
    let context = skill_management_context(filesystem, skill_mounts());

    let result = search_skills(
        &context,
        SkillSearchRequest {
            query: "helper",
            limit: 2,
        },
    )
    .await
    .unwrap();

    assert_eq!(result.skills.len(), 2);
    assert!(result.truncated);
}

#[tokio::test]
async fn search_skills_propagates_filesystem_error() {
    let context =
        skill_management_context_with_root(Arc::new(FailingListFilesystem), skill_mounts());

    let error = search_skills(
        &context,
        SkillSearchRequest {
            query: "helper",
            limit: 10,
        },
    )
    .await
    .unwrap_err();

    assert_eq!(error.kind(), SkillManagementErrorKind::InvalidSkill);
}

#[tokio::test]
async fn search_skills_stops_after_entry_scan_budget() {
    let filesystem = Arc::new(InMemoryBackend::default());
    for index in 0..=250 {
        let name = format!("budget-helper-{index:03}");
        write_file(
            filesystem.as_ref(),
            &format!("/projects/skills/{name}/SKILL.md"),
            skill_md(&name, "budget helper description", "PROMPT"),
        )
        .await;
    }
    let context = skill_management_context(filesystem, skill_mounts());

    let result = search_skills(
        &context,
        SkillSearchRequest {
            query: "budget",
            limit: 1000,
        },
    )
    .await
    .unwrap();

    assert_eq!(result.skills.len(), super::SKILL_SEARCH_ENTRY_SCAN_LIMIT);
    assert!(result.truncated);
}

#[tokio::test]
async fn remove_rejects_system_skill() {
    let filesystem = Arc::new(InMemoryBackend::default());
    write_file(
        filesystem.as_ref(),
        "/projects/system/skills/system-helper/SKILL.md",
        skill_md("system-helper", "system skill description", "PROMPT"),
    )
    .await;
    let context = skill_management_context(filesystem, skill_mounts());

    let error = remove_skill(
        &context,
        SkillRemoveRequest {
            name: "system-helper",
        },
    )
    .await
    .unwrap_err();

    assert_eq!(error.kind(), SkillManagementErrorKind::NotFound);
}

async fn write_file(root: &InMemoryBackend, path: &str, body: String) {
    root.write_file(&VirtualPath::new(path).unwrap(), body.as_bytes())
        .await
        .unwrap();
}

async fn read_file(root: &InMemoryBackend, path: &str) -> String {
    let bytes = root
        .read_file_bounded(
            &VirtualPath::new(path).unwrap(),
            MAX_PROMPT_FILE_SIZE as usize,
        )
        .await
        .unwrap()
        .unwrap();
    String::from_utf8(bytes).unwrap()
}

async fn assert_missing(root: &InMemoryBackend, path: &str) {
    match root
        .read_file_bounded(&VirtualPath::new(path).unwrap(), 1024)
        .await
    {
        Ok(None) | Err(FilesystemError::NotFound { .. }) => {}
        Ok(Some(_)) => panic!("{path} should have been cleaned up"),
        Err(error) => panic!("unexpected filesystem error: {error:?}"),
    }
}

async fn assert_file_contents(root: &InMemoryBackend, path: &str, expected: &[u8]) {
    let bytes = root
        .read_file_bounded(&VirtualPath::new(path).unwrap(), 1024)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("{path} should exist"));
    assert_eq!(bytes, expected);
}

#[derive(Clone)]
struct FailingBundleWriteFilesystem {
    inner: Arc<InMemoryBackend>,
    fail_suffix: &'static str,
    fail_delete: bool,
}

#[async_trait]
impl RootFilesystem for FailingBundleWriteFilesystem {
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn read_file_bounded(
        &self,
        path: &VirtualPath,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, FilesystemError> {
        self.inner.read_file_bounded(path, max_bytes).await
    }

    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        if path.as_str().ends_with(self.fail_suffix) {
            return Err(FilesystemError::Backend {
                operation: FilesystemOperation::WriteFile,
                path: path.clone(),
                reason: "injected bundle write failure".to_string(),
            });
        }
        self.inner.write_file(path, bytes).await
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.create_dir_all(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        if self.fail_delete {
            return Err(FilesystemError::PermissionDenied {
                path: ScopedPath::new(path.as_str().to_string()).unwrap(),
                operation: FilesystemOperation::Delete,
            });
        }
        self.inner.delete(path).await
    }
}

#[derive(Clone)]
struct FailingListFilesystem;

#[async_trait]
impl RootFilesystem for FailingListFilesystem {
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::default()
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        Err(FilesystemError::Backend {
            operation: FilesystemOperation::ListDir,
            path: path.clone(),
            reason: "injected list failure".to_string(),
        })
    }

    async fn list_dir_bounded(
        &self,
        path: &VirtualPath,
        _max_entries: usize,
    ) -> Result<Vec<DirEntry>, FilesystemError> {
        self.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        Err(FilesystemError::Backend {
            operation: FilesystemOperation::Stat,
            path: path.clone(),
            reason: "injected stat failure".to_string(),
        })
    }
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

fn user_skill_mounts() -> MountView {
    MountView::new(vec![MountGrant::new(
        MountAlias::new("/skills").unwrap(),
        VirtualPath::new("/projects/skills").unwrap(),
        MountPermissions::read_write_list_delete(),
    )])
    .unwrap()
}

fn skill_management_context(
    filesystem: Arc<InMemoryBackend>,
    mounts: MountView,
) -> SkillManagementContext {
    let filesystem: Arc<dyn RootFilesystem> = filesystem;
    SkillManagementContext::new(filesystem, mounts, ResourceScope::system())
}

fn skill_management_context_with_root(
    filesystem: Arc<dyn RootFilesystem>,
    mounts: MountView,
) -> SkillManagementContext {
    SkillManagementContext::new(filesystem, mounts, ResourceScope::system())
}

fn skill_md(name: &str, description: &str, prompt: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---\n{prompt}\n")
}
