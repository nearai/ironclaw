//! File-reference resolution and the blueprint lockfile.
//!
//! `text_ref` / `brief_ref` values point at files alongside the blueprint
//! (prompt bodies, mission briefs). The epic requires them to be resolved
//! relative to the blueprint root, read once, and embedded in a lockfile by
//! SHA-256 so an apply is reproducible and tamper-evident across machines.
//!
//! Resolution fails closed twice: absolute paths and any `..` component are
//! rejected lexically before touching the filesystem, and the resolved real
//! path (after following symlinks) must still live under the canonicalized
//! blueprint root — so neither a `..` reference nor a symlink planted inside
//! the blueprint directory can reach outside it. The second check matters for
//! GitOps-style flows where the blueprint directory arrives from a remote
//! repository and may contain hostile symlinks.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::Blueprint;
use crate::error::BlueprintError;

/// One file referenced by the blueprint, with the AST path that referenced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRefSite {
    /// Dotted AST path of the field holding the reference (e.g.
    /// `system_prompt.text_ref`). Used for error messages.
    pub site: String,
    /// The root-relative reference string as written in the blueprint.
    pub reference: String,
}

/// A resolved, hashed file reference recorded in the lockfile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockedFile {
    /// Normalized root-relative path of the referenced file (`/`-separated,
    /// `.` components removed), so equivalent spellings of the same reference
    /// collapse to one entry and the lockfile is identical across platforms.
    pub path: String,
    /// Lowercase hex SHA-256 of the file contents.
    pub sha256: String,
}

/// The blueprint lockfile: the api_version it was produced from plus every
/// referenced file with its content hash, sorted by path for determinism.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lockfile {
    pub api_version: String,
    pub files: Vec<LockedFile>,
}

impl Blueprint {
    /// Collect every file reference in the document, in declaration order.
    pub fn file_refs(&self) -> Vec<FileRefSite> {
        let mut refs = Vec::new();
        if let Some(prompt) = &self.system_prompt {
            refs.push(FileRefSite {
                site: "system_prompt.text_ref".to_string(),
                reference: prompt.text_ref.clone(),
            });
        }
        for (index, mission) in self.missions.iter().enumerate() {
            if let Some(brief) = &mission.brief_ref {
                refs.push(FileRefSite {
                    site: format!("missions[{index}].brief_ref"),
                    reference: brief.clone(),
                });
            }
        }
        if let Some(harness) = &self.harness
            && let Some(inline) = &harness.inline
            && let Some(overlay) = &inline.prompt_overlay
        {
            refs.push(FileRefSite {
                site: "harness.inline.prompt_overlay.text_ref".to_string(),
                reference: overlay.text_ref.clone(),
            });
        }
        refs
    }

    /// Resolve every file reference against `root`, hash the contents, and
    /// produce a [`Lockfile`]. Fails if a reference escapes the root (lexically
    /// or through a symlink) or a referenced file cannot be read.
    pub fn resolve_lockfile(&self, root: &Path) -> Result<Lockfile, BlueprintError> {
        let refs = self.file_refs();
        let mut files = Vec::new();
        if refs.is_empty() {
            return Ok(Lockfile {
                api_version: self.api_version.clone(),
                files,
            });
        }

        // Canonicalize the root once so the containment check below compares
        // real paths even when the root itself is reached through a symlink
        // (e.g. `/var` -> `/private/var` on macOS).
        let canonical_root = root
            .canonicalize()
            .map_err(|e| read_error("(blueprint root)", &root.display().to_string(), &e))?;

        for FileRefSite { site, reference } in refs {
            let (relative, normalized) = validate_relative(&site, &reference)?;
            // The lexical check cannot see symlinks: canonicalize the joined
            // path and require the real file to stay under the root.
            let resolved = canonical_root
                .join(&relative)
                .canonicalize()
                .map_err(|e| read_error(&site, &reference, &e))?;
            if !resolved.starts_with(&canonical_root) {
                return Err(BlueprintError::InvalidFileRef {
                    path: site,
                    reference,
                    reason: "resolves outside the blueprint root via a symlink".to_string(),
                });
            }
            let bytes = std::fs::read(&resolved).map_err(|e| read_error(&site, &reference, &e))?;
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            files.push(LockedFile {
                path: normalized,
                sha256: hex::encode(hasher.finalize()),
            });
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        files.dedup();
        Ok(Lockfile {
            api_version: self.api_version.clone(),
            files,
        })
    }
}

fn read_error(site: &str, reference: &str, error: &std::io::Error) -> BlueprintError {
    BlueprintError::FileRefRead {
        path: site.to_string(),
        reference: reference.to_string(),
        reason: error.to_string(),
    }
}

/// Reject absolute paths, root/prefix components, and any `..` so a reference
/// can only name files at or below the blueprint root. Returns the validated
/// relative path plus its normalized `/`-separated string form for the
/// lockfile (so `files/./a.md` and `files/a.md` record identically).
fn validate_relative(site: &str, reference: &str) -> Result<(PathBuf, String), BlueprintError> {
    let invalid = |reason: &str| BlueprintError::InvalidFileRef {
        path: site.to_string(),
        reference: reference.to_string(),
        reason: reason.to_string(),
    };

    let path = Path::new(reference);
    if path.is_absolute() {
        return Err(invalid("absolute paths are not allowed"));
    }
    let mut normalized = PathBuf::new();
    let mut segments = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => {
                // Lossless: `reference` is `&str`, so every segment is UTF-8.
                segments.push(segment.to_string_lossy());
                normalized.push(segment);
            }
            Component::CurDir => {}
            Component::ParentDir => return Err(invalid("`..` components are not allowed")),
            Component::RootDir | Component::Prefix(_) => {
                return Err(invalid("absolute paths are not allowed"));
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(invalid("empty reference"));
    }
    Ok((normalized, segments.join("/")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_absolute_ref() {
        let err = validate_relative("system_prompt.text_ref", "/etc/passwd")
            .expect_err("absolute rejected");
        assert!(matches!(err, BlueprintError::InvalidFileRef { .. }));
    }

    #[test]
    fn rejects_parent_traversal() {
        let err = validate_relative("system_prompt.text_ref", "../../secrets.txt")
            .expect_err("traversal rejected");
        assert!(matches!(err, BlueprintError::InvalidFileRef { .. }));
    }

    #[test]
    fn accepts_nested_relative() {
        let (resolved, normalized) = validate_relative("system_prompt.text_ref", "files/prompt.md")
            .expect("nested relative ok");
        assert_eq!(resolved, PathBuf::from("files/prompt.md"));
        assert_eq!(normalized, "files/prompt.md");
    }

    #[test]
    fn normalizes_curdir_components() {
        let (resolved, normalized) =
            validate_relative("system_prompt.text_ref", "files/./prompt.md")
                .expect("curdir normalized");
        assert_eq!(resolved, PathBuf::from("files/prompt.md"));
        assert_eq!(
            normalized, "files/prompt.md",
            "equivalent spellings must record identically in the lockfile"
        );
    }
}
