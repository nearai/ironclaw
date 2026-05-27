use std::env;
use std::fs::{self, FileType};
use std::path::{Path, PathBuf};

use ironclaw_skills::{normalize_safe_relative_path, parse_skill_md, validate_skill_name};

type BuildResult<T> = Result<T, Box<dyn std::error::Error>>;

fn main() -> BuildResult<()> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| build_error("ironclaw_reborn_composition lives under crates/"))?;
    embed_reborn_skills(repo_root)
}

fn embed_reborn_skills(repo_root: &Path) -> BuildResult<()> {
    let skills_dir = repo_root.join("skills");
    println!("cargo:rerun-if-changed={}", skills_dir.display());

    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let summaries_out_path = out_dir.join("embedded_reborn_skill_summaries.json");
    let bundles_out_path = out_dir.join("embedded_reborn_skill_bundles.json");
    if !path_is_real_dir(&skills_dir)? {
        fs::write(summaries_out_path, "[]")?;
        fs::write(bundles_out_path, "[]")?;
        return Ok(());
    }

    let mut skill_summaries = Vec::new();
    let mut skill_bundles = Vec::new();
    let mut entries = fs::read_dir(&skills_dir)?.collect::<Result<Vec<_>, _>>()?;
    entries = entries
        .into_iter()
        .filter_map(|entry| match non_symlink_file_type(&entry) {
            Ok(file_type) if file_type.is_dir() => Some(Ok(entry)),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<BuildResult<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let skill_dir = entry.path();
        let skill_md = skill_dir.join("SKILL.md");
        if !path_is_real_file(&skill_md)? {
            continue;
        }

        let dir_name = entry
            .file_name()
            .into_string()
            .map_err(|_| build_error("skill directory name must be UTF-8"))?;
        if !validate_skill_name(&dir_name) {
            return Err(build_error(format!(
                "bundled Reborn skill directory has invalid name `{dir_name}`"
            )));
        }

        let skill_md_content = fs::read_to_string(&skill_md)?;
        let parsed = parse_skill_md(&skill_md_content)
            .map_err(|error| build_error(format!("parse bundled SKILL.md: {error}")))?;
        if parsed.manifest.name != dir_name {
            return Err(build_error(format!(
                "bundled Reborn skill `{}` manifest name `{}` must match directory name",
                dir_name, parsed.manifest.name
            )));
        }

        let files = collect_skill_files(&skill_dir)?;
        skill_summaries.push(serde_json::json!({
            "name": parsed.manifest.name,
            "version": parsed.manifest.version,
            "description": parsed.manifest.description,
            "keywords": parsed.manifest.activation.keywords,
            "tags": parsed.manifest.activation.tags,
            "requires_skills": parsed.manifest.requires.skills,
        }));
        skill_bundles.push(serde_json::json!({
            "name": parsed.manifest.name,
            "files": files,
        }));
    }

    fs::write(summaries_out_path, serde_json::to_string(&skill_summaries)?)?;
    fs::write(bundles_out_path, serde_json::to_string(&skill_bundles)?)?;
    Ok(())
}

fn collect_skill_files(skill_dir: &Path) -> BuildResult<Vec<serde_json::Value>> {
    let mut paths = Vec::new();
    collect_files_recursive(skill_dir, &mut paths)?;
    paths.sort();
    paths
        .into_iter()
        .map(|path| skill_file_json(skill_dir, &path))
        .collect()
}

fn collect_files_recursive(dir: &Path, paths: &mut Vec<PathBuf>) -> BuildResult<()> {
    let mut entries = fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let file_type = non_symlink_file_type(&entry)?;
        if file_type.is_dir() {
            collect_files_recursive(&path, paths)?;
        } else if file_type.is_file() {
            println!("cargo:rerun-if-changed={}", path.display());
            paths.push(path);
        }
    }
    Ok(())
}

fn path_is_real_dir(path: &Path) -> BuildResult<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(build_error(format!(
            "bundled Reborn skills path must not be a symlink: {}",
            path.display()
        ))),
        Ok(metadata) => Ok(metadata.is_dir()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn path_is_real_file(path: &Path) -> BuildResult<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(build_error(format!(
            "bundled Reborn skill file must not be a symlink: {}",
            path.display()
        ))),
        Ok(metadata) => Ok(metadata.is_file()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn non_symlink_file_type(entry: &fs::DirEntry) -> BuildResult<FileType> {
    let file_type = entry.file_type()?;
    if file_type.is_symlink() {
        return Err(build_error(format!(
            "bundled Reborn skill entry must not be a symlink: {}",
            entry.path().display()
        )));
    }
    Ok(file_type)
}

fn skill_file_json(skill_dir: &Path, source_path: &Path) -> BuildResult<serde_json::Value> {
    let relative_path = source_path.strip_prefix(skill_dir)?;
    let normalized = normalize_safe_relative_path(relative_path)
        .map_err(|error| build_error(format!("skill bundle file path must be safe: {error:?}")))?;
    let path = normalized
        .to_str()
        .ok_or_else(|| build_error("skill bundle file path must be UTF-8"))?
        .replace('\\', "/");
    let bytes = fs::read(source_path)?;
    Ok(serde_json::json!({
        "path": path,
        "bytes": bytes,
    }))
}

fn build_error(reason: impl Into<String>) -> Box<dyn std::error::Error> {
    Box::new(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        reason.into(),
    ))
}
