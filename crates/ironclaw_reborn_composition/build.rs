use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use ironclaw_skills::{normalize_safe_relative_path, parse_skill_md, validate_skill_name};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("ironclaw_reborn_composition lives under crates/");
    embed_reborn_skills(repo_root);
}

fn embed_reborn_skills(repo_root: &Path) {
    let skills_dir = repo_root.join("skills");
    println!("cargo:rerun-if-changed={}", skills_dir.display());

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("out dir"));
    let out_path = out_dir.join("embedded_reborn_skills.json");
    if !skills_dir.is_dir() {
        fs::write(out_path, "[]").expect("write empty embedded skills");
        return;
    }

    let mut skill_entries = Vec::new();
    let mut entries = fs::read_dir(&skills_dir)
        .expect("read skills dir")
        .collect::<Result<Vec<_>, _>>()
        .expect("read skills dir entries");
    entries.retain(|entry| entry.path().is_dir());
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let skill_dir = entry.path();
        let skill_md = skill_dir.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }

        let dir_name = entry
            .file_name()
            .into_string()
            .expect("skill directory name must be UTF-8");
        if !validate_skill_name(&dir_name) {
            panic!("bundled Reborn skill directory has invalid name `{dir_name}`");
        }

        let skill_md_content = fs::read_to_string(&skill_md).expect("read bundled SKILL.md");
        let parsed = parse_skill_md(&skill_md_content).expect("parse bundled SKILL.md");
        if parsed.manifest.name != dir_name {
            panic!(
                "bundled Reborn skill `{}` manifest name `{}` must match directory name",
                dir_name, parsed.manifest.name
            );
        }

        let files = collect_skill_files(&skill_dir);
        skill_entries.push(serde_json::json!({
            "name": parsed.manifest.name,
            "version": parsed.manifest.version,
            "description": parsed.manifest.description,
            "keywords": parsed.manifest.activation.keywords,
            "tags": parsed.manifest.activation.tags,
            "requires_skills": parsed.manifest.requires.skills,
            "files": files,
        }));
    }

    fs::write(
        out_path,
        serde_json::to_string(&skill_entries).expect("serialize embedded skills"),
    )
    .expect("write embedded skills");
}

fn collect_skill_files(skill_dir: &Path) -> Vec<serde_json::Value> {
    let mut paths = Vec::new();
    collect_files_recursive(skill_dir, &mut paths);
    paths.sort();
    paths
        .into_iter()
        .map(|path| skill_file_json(skill_dir, &path))
        .collect()
}

fn collect_files_recursive(dir: &Path, paths: &mut Vec<PathBuf>) {
    let mut entries = fs::read_dir(dir)
        .expect("read skill bundle dir")
        .collect::<Result<Vec<_>, _>>()
        .expect("read skill bundle entries");
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive(&path, paths);
        } else if path.is_file() {
            println!("cargo:rerun-if-changed={}", path.display());
            paths.push(path);
        }
    }
}

fn skill_file_json(skill_dir: &Path, source_path: &Path) -> serde_json::Value {
    let relative_path = source_path
        .strip_prefix(skill_dir)
        .expect("skill file under root");
    let normalized =
        normalize_safe_relative_path(relative_path).expect("skill bundle file path must be safe");
    let path = normalized
        .to_str()
        .expect("skill bundle file path must be UTF-8")
        .replace('\\', "/");
    let bytes = fs::read(source_path).expect("read skill bundle file");
    serde_json::json!({
        "path": path,
        "bytes": bytes,
    })
}
