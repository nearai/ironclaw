use std::path::{Component, Path, PathBuf};

use ironclaw_host_api::dispatch::RuntimeDispatchErrorKind;
use ironclaw_skills::normalize_safe_relative_path;

use crate::skills::SkillManagementCapabilityError;

use super::{SkillUrlPayload, SkillUrlPayloadFile, MAX_TOTAL_UNZIPPED_BYTES, MAX_ZIP_ENTRY_BYTES};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SkillBundle {
    pub(super) skill_md: String,
    pub(super) files: Vec<SkillUrlPayloadFile>,
    pub(super) bundle_subdir: Option<String>,
}

pub(super) struct BundleCollector {
    root: PathBuf,
    skill_md: Option<String>,
    files: Vec<SkillUrlPayloadFile>,
    total_bytes: u64,
}

impl BundleCollector {
    pub(super) fn new(root: PathBuf) -> Self {
        Self {
            root,
            skill_md: None,
            files: Vec::new(),
            total_bytes: 0,
        }
    }

    pub(super) fn push_file(
        &mut self,
        path: PathBuf,
        bytes: Vec<u8>,
    ) -> Result<(), SkillManagementCapabilityError> {
        if bytes.len() as u64 > MAX_ZIP_ENTRY_BYTES {
            return Err(SkillManagementCapabilityError::new(
                RuntimeDispatchErrorKind::OutputTooLarge,
            ));
        }
        self.total_bytes = self
            .total_bytes
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| {
                SkillManagementCapabilityError::new(RuntimeDispatchErrorKind::OutputTooLarge)
            })?;
        if self.total_bytes > MAX_TOTAL_UNZIPPED_BYTES {
            return Err(SkillManagementCapabilityError::new(
                RuntimeDispatchErrorKind::OutputTooLarge,
            ));
        }

        let Some(relative) = self.relative_path(&path)? else {
            return Ok(());
        };
        if relative == Path::new("SKILL.md") {
            if bytes.len() as u64 > ironclaw_skills::MAX_PROMPT_FILE_SIZE {
                return Err(SkillManagementCapabilityError::new(
                    RuntimeDispatchErrorKind::OutputTooLarge,
                ));
            }
            self.skill_md = Some(String::from_utf8(bytes).map_err(|_| {
                SkillManagementCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
            })?);
        } else {
            if self.files.len() >= ironclaw_skills::MAX_INSTALL_BUNDLE_FILES {
                return Err(SkillManagementCapabilityError::new(
                    RuntimeDispatchErrorKind::OutputTooLarge,
                ));
            }
            self.files.push(SkillUrlPayloadFile {
                path: relative.to_path_buf(),
                contents: bytes,
            });
        }
        Ok(())
    }

    pub(super) fn relative_path(
        &self,
        path: &Path,
    ) -> Result<Option<PathBuf>, SkillManagementCapabilityError> {
        let relative = path.strip_prefix(&self.root).map_err(|_| {
            SkillManagementCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
        })?;
        if relative.as_os_str().is_empty() {
            return Ok(None);
        }
        Ok(Some(relative.to_path_buf()))
    }

    pub(super) fn finish(self) -> Result<SkillUrlPayload, SkillManagementCapabilityError> {
        let content = self.skill_md.ok_or_else(|| {
            SkillManagementCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
        })?;
        Ok(SkillUrlPayload {
            content,
            files: self.files,
        })
    }
}

pub(super) fn normalize_archive_path(
    path: &Path,
) -> Result<PathBuf, SkillManagementCapabilityError> {
    normalize_safe_relative_path(path)
        .map_err(|_| SkillManagementCapabilityError::new(RuntimeDispatchErrorKind::InputEncode))
}

pub(super) fn strip_common_archive_root(paths: &[PathBuf]) -> Option<PathBuf> {
    let mut root: Option<std::ffi::OsString> = None;
    let mut has_nested = false;
    for path in paths {
        let mut components = path.components();
        let Some(Component::Normal(first)) = components.next() else {
            return None;
        };
        has_nested |= components.next().is_some();
        match &root {
            Some(existing) if existing != first => return None,
            None => root = Some(first.to_os_string()),
            _ => {}
        }
    }
    if !has_nested {
        return None;
    }
    root.map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_archive_path_rejects_absolute() {
        let result = normalize_archive_path(Path::new("/etc/passwd"));
        assert_eq!(
            result.unwrap_err().kind(),
            RuntimeDispatchErrorKind::InputEncode
        );
    }

    #[test]
    fn normalize_archive_path_rejects_upward_traversal() {
        let result = normalize_archive_path(Path::new("../escape"));
        assert_eq!(
            result.unwrap_err().kind(),
            RuntimeDispatchErrorKind::InputEncode
        );
    }

    #[test]
    fn normalize_archive_path_normalizes_dot_segments() {
        let path = normalize_archive_path(Path::new("foo/./bar/./baz")).unwrap();
        assert_eq!(path, PathBuf::from("foo/bar/baz"));
    }

    #[test]
    fn normalize_archive_path_accepts_simple_relative() {
        let path = normalize_archive_path(Path::new("my-skill/SKILL.md")).unwrap();
        assert_eq!(path, PathBuf::from("my-skill/SKILL.md"));
    }

    #[test]
    fn normalize_archive_path_rejects_windows_backslash() {
        let result = normalize_archive_path(Path::new("my-skill\\SKILL.md"));
        assert_eq!(
            result.unwrap_err().kind(),
            RuntimeDispatchErrorKind::InputEncode
        );
    }

    #[test]
    fn normalize_archive_path_rejects_empty() {
        let result = normalize_archive_path(Path::new(""));
        assert_eq!(
            result.unwrap_err().kind(),
            RuntimeDispatchErrorKind::InputEncode
        );
    }

    #[test]
    fn strip_common_root_single_file_no_strip() {
        let paths = vec![PathBuf::from("SKILL.md")];
        assert_eq!(strip_common_archive_root(&paths), None);
    }

    #[test]
    fn strip_common_root_two_flat_files_no_strip() {
        let paths = vec![PathBuf::from("SKILL.md"), PathBuf::from("config.json")];
        assert_eq!(strip_common_archive_root(&paths), None);
    }

    #[test]
    fn strip_common_root_nested_with_common_prefix() {
        let paths = vec![
            PathBuf::from("my-skill-repo/SKILL.md"),
            PathBuf::from("my-skill-repo/tools/helper.rs"),
        ];
        assert_eq!(
            strip_common_archive_root(&paths).as_deref(),
            Some(Path::new("my-skill-repo"))
        );
    }

    #[test]
    fn strip_common_root_no_common_prefix() {
        let paths = vec![
            PathBuf::from("repo-a/SKILL.md"),
            PathBuf::from("repo-b/SKILL.md"),
        ];
        assert_eq!(strip_common_archive_root(&paths), None);
    }

    #[test]
    fn strip_common_root_absolute_components_return_none() {
        let paths = vec![PathBuf::from("/absolute/SKILL.md")];
        assert_eq!(strip_common_archive_root(&paths), None);
    }

    #[test]
    fn bundle_collector_new_accepts_root_path() {
        let collector = BundleCollector::new(PathBuf::from("my-skill"));
        assert_eq!(collector.root, PathBuf::from("my-skill"));
        assert_eq!(collector.skill_md, None);
        assert!(collector.files.is_empty());
        assert_eq!(collector.total_bytes, 0);
    }

    #[test]
    fn bundle_collector_relative_path_returns_relative() {
        let collector = BundleCollector::new(PathBuf::from("root"));
        let rel = collector
            .relative_path(Path::new("root/sub/file.txt"))
            .unwrap()
            .unwrap();
        assert_eq!(rel, PathBuf::from("sub/file.txt"));
    }

    #[test]
    fn bundle_collector_relative_path_returns_none_for_root_itself() {
        let collector = BundleCollector::new(PathBuf::from("root"));
        let rel = collector.relative_path(Path::new("root")).unwrap();
        assert_eq!(rel, None);
    }

    #[test]
    fn bundle_collector_relative_path_errors_on_mismatch() {
        let collector = BundleCollector::new(PathBuf::from("root"));
        let err = collector
            .relative_path(Path::new("other/file.txt"))
            .unwrap_err();
        assert_eq!(err.kind(), RuntimeDispatchErrorKind::OperationFailed);
    }

    #[test]
    fn bundle_collector_push_skill_md_then_finish() {
        let mut collector = BundleCollector::new(PathBuf::from("skill"));
        collector
            .push_file(
                PathBuf::from("skill/SKILL.md"),
                b"# Test Skill\n
## Description\nA test."
                    .to_vec(),
            )
            .unwrap();
        let payload = collector.finish().unwrap();
        assert_eq!(
            payload.content,
            "# Test Skill\n
## Description\nA test."
        );
        assert!(payload.files.is_empty());
    }

    #[test]
    fn bundle_collector_push_extra_files() {
        let mut collector = BundleCollector::new(PathBuf::from("skill"));
        collector
            .push_file(PathBuf::from("skill/SKILL.md"), b"content".to_vec())
            .unwrap();
        collector
            .push_file(PathBuf::from("skill/config.json"), b"{}".to_vec())
            .unwrap();
        collector
            .push_file(
                PathBuf::from("skill/scripts/setup.sh"),
                b"#!/bin/sh".to_vec(),
            )
            .unwrap();
        let payload = collector.finish().unwrap();
        assert_eq!(payload.files.len(), 2);
        assert!(payload
            .files
            .iter()
            .any(|f| f.path == PathBuf::from("config.json")));
        assert!(payload
            .files
            .iter()
            .any(|f| f.path == PathBuf::from("scripts/setup.sh")));
    }

    #[test]
    fn bundle_collector_finish_errors_without_skill_md() {
        let collector = BundleCollector::new(PathBuf::from("skill"));
        let err = collector.finish().unwrap_err();
        assert_eq!(err.kind(), RuntimeDispatchErrorKind::OperationFailed);
    }

    #[test]
    fn bundle_collector_rejects_entry_over_limit() {
        let mut collector = BundleCollector::new(PathBuf::from("skill"));
        let oversized = vec![0u8; (MAX_ZIP_ENTRY_BYTES + 1) as usize];
        let err = collector
            .push_file(PathBuf::from("skill/big.bin"), oversized)
            .unwrap_err();
        assert_eq!(err.kind(), RuntimeDispatchErrorKind::OutputTooLarge);
    }

    #[test]
    fn bundle_collector_rejects_total_over_limit() {
        let mut collector = BundleCollector::new(PathBuf::from("skill"));
        // Push SKILL.md first to avoid finish() error
        collector
            .push_file(PathBuf::from("skill/SKILL.md"), b"valid".to_vec())
            .unwrap();
        // Push a file that would exceed total
        let big = vec![0u8; (MAX_TOTAL_UNZIPPED_BYTES + 1) as usize];
        let err = collector
            .push_file(PathBuf::from("skill/big.bin"), big)
            .unwrap_err();
        assert_eq!(err.kind(), RuntimeDispatchErrorKind::OutputTooLarge);
    }

    #[test]
    fn bundle_collector_rejects_invalid_utf8_in_skill_md() {
        let mut collector = BundleCollector::new(PathBuf::from("skill"));
        let err = collector
            .push_file(
                PathBuf::from("skill/SKILL.md"),
                vec![0xff, 0xfe, 0x00, 0x01],
            )
            .unwrap_err();
        assert_eq!(err.kind(), RuntimeDispatchErrorKind::OperationFailed);
    }

    #[test]
    fn bundle_collector_ignore_files_outside_root() {
        let mut collector = BundleCollector::new(PathBuf::from("subdir"));
        collector
            .push_file(PathBuf::from("subdir/SKILL.md"), b"content".to_vec())
            .unwrap();
        let payload = collector.finish().unwrap();
        assert_eq!(payload.content, "content");
    }

    #[test]
    fn strip_common_root_single_nested_file_no_strip() {
        // One deeply nested file: the root is the parent, but has_nested is false
        let paths = vec![PathBuf::from("my-skill-repo/SKILL.md")];
        assert_eq!(strip_common_archive_root(&paths), None);
    }

    #[test]
    fn strip_common_root_mixed_prefixes_return_none() {
        let paths = vec![
            PathBuf::from("v1-skill/SKILL.md"),
            PathBuf::from("v1-skill/helper.txt"),
            PathBuf::from("v2-skill/README.md"),
        ];
        assert_eq!(strip_common_archive_root(&paths), None);
    }
}
