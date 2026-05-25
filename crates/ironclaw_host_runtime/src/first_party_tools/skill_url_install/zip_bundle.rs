use std::{collections::HashSet, io::Read, path::Path};

use ironclaw_host_api::RuntimeDispatchErrorKind;

use crate::FirstPartyCapabilityError;

use super::{
    MAX_TOTAL_UNZIPPED_BYTES, MAX_ZIP_ENTRY_BYTES, MAX_ZIP_FILE_ENTRIES, SkillUrlPayloadFile,
    bundle::{SkillBundle, normalize_archive_path, strip_common_archive_root},
};

pub(super) async fn extract_skill_bundle_blocking(
    data: Vec<u8>,
    requested_subdir: Option<String>,
) -> Result<SkillBundle, FirstPartyCapabilityError> {
    tokio::task::spawn_blocking(move || extract_skill_bundle(&data, requested_subdir.as_deref()))
        .await
        .map_err(|error| {
            if error.is_panic() {
                tracing::error!("skill URL ZIP extraction worker panicked");
            }
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Backend)
        })?
}

pub(super) fn extract_skill_bundle(
    data: &[u8],
    requested_subdir: Option<&str>,
) -> Result<SkillBundle, FirstPartyCapabilityError> {
    let reader = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed))?;

    let mut raw_paths = Vec::new();
    for index in 0..archive.len() {
        let file = archive.by_index(index).map_err(|_| {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
        })?;
        if !file.is_dir() {
            if raw_paths.len() >= MAX_ZIP_FILE_ENTRIES {
                return Err(FirstPartyCapabilityError::new(
                    RuntimeDispatchErrorKind::OutputTooLarge,
                ));
            }
            raw_paths.push(normalize_archive_path(Path::new(file.name()))?);
        }
    }
    let strip_root = strip_common_archive_root(&raw_paths);
    let mut files = Vec::<(std::path::PathBuf, Vec<u8>)>::new();
    let mut seen_paths = HashSet::<std::path::PathBuf>::new();
    let mut skill_dirs = HashSet::<std::path::PathBuf>::new();
    let mut total_unzipped_bytes = 0u64;

    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|_| {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
        })?;
        if file.is_dir() {
            continue;
        }
        if file.size() > MAX_ZIP_ENTRY_BYTES {
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::OutputTooLarge,
            ));
        }
        let entry_name = file.name().to_string();
        let mut path = normalize_archive_path(Path::new(&entry_name))?;
        if let Some(root) = &strip_root
            && let Ok(stripped) = path.strip_prefix(root)
        {
            path = stripped.to_path_buf();
        }
        if path.as_os_str().is_empty() {
            continue;
        }
        if !seen_paths.insert(path.clone()) {
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::InputEncode,
            ));
        }

        let mut contents = Vec::new();
        (&mut file)
            .take(MAX_ZIP_ENTRY_BYTES + 1)
            .read_to_end(&mut contents)
            .map_err(|_| {
                FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
            })?;
        if contents.len() as u64 > MAX_ZIP_ENTRY_BYTES {
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::OutputTooLarge,
            ));
        }
        total_unzipped_bytes = total_unzipped_bytes
            .checked_add(contents.len() as u64)
            .ok_or_else(|| {
                FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OutputTooLarge)
            })?;
        if total_unzipped_bytes > MAX_TOTAL_UNZIPPED_BYTES {
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::OutputTooLarge,
            ));
        }
        if path.file_name().is_some_and(|name| name == "SKILL.md") {
            skill_dirs.insert(path.parent().unwrap_or(Path::new("")).to_path_buf());
        }
        files.push((path, contents));
    }

    let requested_dir = if let Some(subdir) = requested_subdir {
        let normalized = normalize_archive_path(Path::new(subdir))?;
        if !skill_dirs.contains(&normalized) {
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::OperationFailed,
            ));
        }
        normalized
    } else {
        match skill_dirs.len() {
            0 => {
                return Err(FirstPartyCapabilityError::new(
                    RuntimeDispatchErrorKind::OperationFailed,
                ));
            }
            1 => skill_dirs.into_iter().next().unwrap_or_default(),
            _ => {
                return Err(FirstPartyCapabilityError::new(
                    RuntimeDispatchErrorKind::InputEncode,
                ));
            }
        }
    };

    let mut skill_md = None;
    let mut extra_files = Vec::new();
    for (path, contents) in files {
        let Ok(relative) = path.strip_prefix(&requested_dir) else {
            continue;
        };
        if relative.as_os_str().is_empty() {
            continue;
        }
        if relative == Path::new("SKILL.md") {
            if contents.len() as u64 > ironclaw_skills::MAX_PROMPT_FILE_SIZE {
                return Err(FirstPartyCapabilityError::new(
                    RuntimeDispatchErrorKind::OutputTooLarge,
                ));
            }
            skill_md = Some(String::from_utf8(contents).map_err(|_| {
                FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
            })?);
            continue;
        }
        extra_files.push(SkillUrlPayloadFile {
            path: relative.to_path_buf(),
            contents,
        });
        if extra_files.len() > ironclaw_skills::MAX_INSTALL_BUNDLE_FILES {
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::OutputTooLarge,
            ));
        }
    }

    let skill_md = skill_md
        .ok_or_else(|| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed))?;
    Ok(SkillBundle {
        skill_md,
        files: extra_files,
        bundle_subdir: (!requested_dir.as_os_str().is_empty())
            .then(|| requested_dir.display().to_string()),
    })
}
