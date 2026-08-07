use std::{collections::HashSet, io::Read, path::Path};

use ironclaw_host_api::dispatch::RuntimeDispatchErrorKind;

use crate::skills::SkillManagementCapabilityError;

use super::{
    MAX_TOTAL_UNZIPPED_BYTES, MAX_ZIP_ENTRY_BYTES, MAX_ZIP_FILE_ENTRIES, SkillUrlPayloadFile,
    bundle::{SkillBundle, normalize_archive_path, strip_common_archive_root},
};

pub(super) async fn extract_skill_bundle_blocking(
    data: Vec<u8>,
    requested_subdir: Option<String>,
) -> Result<SkillBundle, SkillManagementCapabilityError> {
    tokio::task::spawn_blocking(move || extract_skill_bundle(&data, requested_subdir.as_deref()))
        .await
        .map_err(|error| {
            if error.is_panic() {
                tracing::error!("skill URL ZIP extraction worker panicked");
            }
            SkillManagementCapabilityError::new(RuntimeDispatchErrorKind::Backend)
        })?
}

pub(super) fn extract_skill_bundle(
    data: &[u8],
    requested_subdir: Option<&str>,
) -> Result<SkillBundle, SkillManagementCapabilityError> {
    let reader = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(reader).map_err(|_| {
        SkillManagementCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
    })?;
    if archive.len() > MAX_ZIP_FILE_ENTRIES {
        return Err(SkillManagementCapabilityError::new(
            RuntimeDispatchErrorKind::OutputTooLarge,
        ));
    }

    let mut raw_paths = Vec::new();
    for index in 0..archive.len() {
        let file = archive.by_index(index).map_err(|_| {
            SkillManagementCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
        })?;
        if !file.is_dir() {
            if raw_paths.len() >= MAX_ZIP_FILE_ENTRIES {
                return Err(SkillManagementCapabilityError::new(
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
            SkillManagementCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
        })?;
        if file.is_dir() {
            continue;
        }
        if file.size() > MAX_ZIP_ENTRY_BYTES {
            return Err(SkillManagementCapabilityError::new(
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
            return Err(SkillManagementCapabilityError::new(
                RuntimeDispatchErrorKind::InputEncode,
            ));
        }

        let mut contents = Vec::new();
        (&mut file)
            .take(MAX_ZIP_ENTRY_BYTES + 1)
            .read_to_end(&mut contents)
            .map_err(|_| {
                SkillManagementCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
            })?;
        if contents.len() as u64 > MAX_ZIP_ENTRY_BYTES {
            return Err(SkillManagementCapabilityError::new(
                RuntimeDispatchErrorKind::OutputTooLarge,
            ));
        }
        total_unzipped_bytes = total_unzipped_bytes
            .checked_add(contents.len() as u64)
            .ok_or_else(|| {
                SkillManagementCapabilityError::new(RuntimeDispatchErrorKind::OutputTooLarge)
            })?;
        if total_unzipped_bytes > MAX_TOTAL_UNZIPPED_BYTES {
            return Err(SkillManagementCapabilityError::new(
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
            return Err(SkillManagementCapabilityError::new(
                RuntimeDispatchErrorKind::OperationFailed,
            ));
        }
        normalized
    } else {
        match skill_dirs.len() {
            0 => {
                return Err(SkillManagementCapabilityError::new(
                    RuntimeDispatchErrorKind::OperationFailed,
                ));
            }
            1 => skill_dirs.into_iter().next().unwrap_or_default(),
            _ => {
                return Err(SkillManagementCapabilityError::new(
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
                return Err(SkillManagementCapabilityError::new(
                    RuntimeDispatchErrorKind::OutputTooLarge,
                ));
            }
            skill_md = Some(String::from_utf8(contents).map_err(|_| {
                SkillManagementCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
            })?);
            continue;
        }
        if extra_files.len() >= ironclaw_skills::MAX_INSTALL_BUNDLE_FILES {
            return Err(SkillManagementCapabilityError::new(
                RuntimeDispatchErrorKind::OutputTooLarge,
            ));
        }
        extra_files.push(SkillUrlPayloadFile {
            path: relative.to_path_buf(),
            contents,
        });
    }

    let skill_md = skill_md.ok_or_else(|| {
        SkillManagementCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
    })?;
    Ok(SkillBundle {
        skill_md,
        files: extra_files,
        bundle_subdir: (!requested_dir.as_os_str().is_empty())
            .then(|| requested_dir.display().to_string()),
    })
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::PathBuf;

    use super::*;

    fn skill_bundle_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        skill_bundle_zip_owned(
            files
                .iter()
                .map(|(path, content)| ((*path).to_string(), (*content).to_vec())),
        )
    }

    fn skill_bundle_zip_owned(files: impl IntoIterator<Item = (String, Vec<u8>)>) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (path, content) in files {
            writer.start_file(path, options).unwrap();
            writer.write_all(&content).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn extract_flat_single_skill_md() {
        let data = skill_bundle_zip(&[("SKILL.md", b"# One\n")]);
        let bundle = extract_skill_bundle(&data, None).unwrap();
        assert_eq!(bundle.skill_md, "# One\n");
        assert!(bundle.files.is_empty());
        assert_eq!(bundle.bundle_subdir, None);
    }

    #[test]
    fn extract_common_root_skill_with_supporting_files() {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        writer.add_directory("my-skill-repo/", options).unwrap();
        writer
            .start_file("my-skill-repo/SKILL.md", options)
            .unwrap();
        writer.write_all(b"# Root\n").unwrap();
        writer
            .start_file("my-skill-repo/tools/helper.rs", options)
            .unwrap();
        writer.write_all(b"fn help() {}").unwrap();
        let data = writer.finish().unwrap().into_inner();

        let bundle = extract_skill_bundle(&data, None).unwrap();
        assert_eq!(bundle.skill_md, "# Root\n");
        // The common archive root is stripped, leaving the skill at bundle
        // root, so no subdir is recorded.
        assert_eq!(bundle.bundle_subdir, None);
        assert_eq!(bundle.files.len(), 1);
        assert_eq!(bundle.files[0].path, PathBuf::from("tools/helper.rs"));
    }

    #[test]
    fn extract_records_subdir_after_stripping_archive_root() {
        let data = skill_bundle_zip(&[
            ("repo/pack/SKILL.md", b"# Packed\n"),
            ("repo/pack/run.sh", b"#!/bin/sh"),
        ]);
        let bundle = extract_skill_bundle(&data, None).unwrap();
        assert_eq!(bundle.skill_md, "# Packed\n");
        assert_eq!(bundle.bundle_subdir.as_deref(), Some("pack"));
        assert_eq!(bundle.files.len(), 1);
        assert_eq!(bundle.files[0].path, PathBuf::from("run.sh"));
    }

    #[test]
    fn extract_rejects_traversal_entries() {
        let data = skill_bundle_zip(&[("SKILL.md", b"# ok\n"), ("../escape.txt", b"evil")]);
        let err = extract_skill_bundle(&data, None).unwrap_err();
        assert_eq!(err.kind(), RuntimeDispatchErrorKind::InputEncode);
    }

    #[test]
    fn extract_rejects_absolute_entries() {
        let data = skill_bundle_zip(&[("/etc/passwd", b"root:x")]);
        let err = extract_skill_bundle(&data, None).unwrap_err();
        assert_eq!(err.kind(), RuntimeDispatchErrorKind::InputEncode);
    }

    #[test]
    fn extract_rejects_duplicate_normalized_paths() {
        // Distinct raw entry names that normalize to the same path are
        // ambiguous and rejected rather than silently overwriting.
        let data = skill_bundle_zip(&[
            ("repo/SKILL.md", b"# One\n"),
            ("repo/./SKILL.md", b"# Two\n"),
        ]);
        let err = extract_skill_bundle(&data, None).unwrap_err();
        assert_eq!(err.kind(), RuntimeDispatchErrorKind::InputEncode);
    }

    #[test]
    fn extract_errors_without_skill_md() {
        let data = skill_bundle_zip(&[("readme.txt", b"no skill here")]);
        let err = extract_skill_bundle(&data, None).unwrap_err();
        assert_eq!(err.kind(), RuntimeDispatchErrorKind::OperationFailed);
    }

    #[test]
    fn extract_rejects_multiple_skill_dirs_without_subdir() {
        let data = skill_bundle_zip(&[("alpha/SKILL.md", b"# A\n"), ("beta/SKILL.md", b"# B\n")]);
        let err = extract_skill_bundle(&data, None).unwrap_err();
        assert_eq!(err.kind(), RuntimeDispatchErrorKind::InputEncode);
    }

    #[test]
    fn extract_selects_requested_subdir() {
        let data = skill_bundle_zip(&[
            ("alpha/SKILL.md", b"# A\n"),
            ("beta/SKILL.md", b"# B\n"),
            ("beta/tool.sh", b"#!/bin/sh"),
        ]);
        let bundle = extract_skill_bundle(&data, Some("beta")).unwrap();
        assert_eq!(bundle.skill_md, "# B\n");
        assert_eq!(bundle.bundle_subdir.as_deref(), Some("beta"));
        assert_eq!(bundle.files.len(), 1);
        assert_eq!(bundle.files[0].path, PathBuf::from("tool.sh"));
    }

    #[test]
    fn extract_rejects_missing_requested_subdir() {
        let data = skill_bundle_zip(&[("alpha/SKILL.md", b"# A\n")]);
        let err = extract_skill_bundle(&data, Some("nope")).unwrap_err();
        assert_eq!(err.kind(), RuntimeDispatchErrorKind::OperationFailed);
    }

    #[test]
    fn extract_rejects_invalid_zip_bytes() {
        let err = extract_skill_bundle(b"not a zip at all", None).unwrap_err();
        assert_eq!(err.kind(), RuntimeDispatchErrorKind::OperationFailed);
    }

    #[test]
    fn extract_rejects_skill_md_over_prompt_size() {
        let oversized = vec![b'x'; ironclaw_skills::MAX_PROMPT_FILE_SIZE as usize + 1];
        let data = skill_bundle_zip(&[("SKILL.md", &oversized)]);
        let err = extract_skill_bundle(&data, None).unwrap_err();
        assert_eq!(err.kind(), RuntimeDispatchErrorKind::OutputTooLarge);
    }

    #[test]
    fn extract_rejects_entry_over_zip_entry_limit() {
        let oversized = vec![0u8; MAX_ZIP_ENTRY_BYTES as usize + 1];
        let data = skill_bundle_zip(&[("big.bin", &oversized)]);
        let err = extract_skill_bundle(&data, None).unwrap_err();
        assert_eq!(err.kind(), RuntimeDispatchErrorKind::OutputTooLarge);
    }

    #[test]
    fn extract_rejects_total_unzipped_over_limit() {
        // Each entry sits exactly at the per-entry limit, so only the running
        // total crosses MAX_TOTAL_UNZIPPED_BYTES (10 full-size entries fit).
        let chunks = MAX_TOTAL_UNZIPPED_BYTES / MAX_ZIP_ENTRY_BYTES;
        let files = (0..(chunks + 2)).map(|index| {
            (
                format!("chunk-{index}.bin"),
                vec![0u8; MAX_ZIP_ENTRY_BYTES as usize],
            )
        });
        let data = skill_bundle_zip_owned(files);
        let err = extract_skill_bundle(&data, None).unwrap_err();
        assert_eq!(err.kind(), RuntimeDispatchErrorKind::OutputTooLarge);
    }

    #[test]
    fn extract_rejects_too_many_zip_entries() {
        let files = (0..(MAX_ZIP_FILE_ENTRIES + 1))
            .map(|index| (format!("entry-{index}.txt"), b"x".to_vec()));
        let data = skill_bundle_zip_owned(files);
        let err = extract_skill_bundle(&data, None).unwrap_err();
        assert_eq!(err.kind(), RuntimeDispatchErrorKind::OutputTooLarge);
    }
}
