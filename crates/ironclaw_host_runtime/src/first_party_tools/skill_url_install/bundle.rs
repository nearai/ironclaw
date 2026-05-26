use std::path::{Component, Path, PathBuf};

use ironclaw_host_api::RuntimeDispatchErrorKind;
use ironclaw_skills::normalize_safe_relative_path;

use crate::FirstPartyCapabilityError;

use super::{MAX_TOTAL_UNZIPPED_BYTES, MAX_ZIP_ENTRY_BYTES, SkillUrlPayload, SkillUrlPayloadFile};

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
    ) -> Result<(), FirstPartyCapabilityError> {
        if bytes.len() as u64 > MAX_ZIP_ENTRY_BYTES {
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::OutputTooLarge,
            ));
        }
        self.total_bytes = self
            .total_bytes
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| {
                FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OutputTooLarge)
            })?;
        if self.total_bytes > MAX_TOTAL_UNZIPPED_BYTES {
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::OutputTooLarge,
            ));
        }

        let Some(relative) = self.relative_path(&path)? else {
            return Ok(());
        };
        if relative == Path::new("SKILL.md") {
            if bytes.len() as u64 > ironclaw_skills::MAX_PROMPT_FILE_SIZE {
                return Err(FirstPartyCapabilityError::new(
                    RuntimeDispatchErrorKind::OutputTooLarge,
                ));
            }
            self.skill_md = Some(String::from_utf8(bytes).map_err(|_| {
                FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
            })?);
        } else {
            if self.files.len() >= ironclaw_skills::MAX_INSTALL_BUNDLE_FILES {
                return Err(FirstPartyCapabilityError::new(
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
    ) -> Result<Option<PathBuf>, FirstPartyCapabilityError> {
        let relative = path.strip_prefix(&self.root).map_err(|_| {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
        })?;
        if relative.as_os_str().is_empty() {
            return Ok(None);
        }
        Ok(Some(relative.to_path_buf()))
    }

    pub(super) fn finish(self) -> Result<SkillUrlPayload, FirstPartyCapabilityError> {
        let content = self.skill_md.ok_or_else(|| {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
        })?;
        Ok(SkillUrlPayload {
            content,
            files: self.files,
        })
    }
}

pub(super) fn normalize_archive_path(path: &Path) -> Result<PathBuf, FirstPartyCapabilityError> {
    normalize_safe_relative_path(path)
        .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::InputEncode))
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
