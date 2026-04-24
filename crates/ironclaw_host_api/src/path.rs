use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::HostApiError;

/// Physical host/backend path. This type is intentionally not serializable.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HostPath(PathBuf);

impl HostPath {
    pub fn from_path_buf(path: PathBuf) -> Self {
        Self(path)
    }

    pub fn as_path(&self) -> &std::path::Path {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VirtualPath(String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ScopedPath(String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MountAlias(String);

const VIRTUAL_ROOTS: &[&str] = &[
    "/engine",
    "/system/extensions",
    "/users",
    "/projects",
    "/memory",
];

const RAW_HOST_PREFIXES: &[&str] = &[
    "/Users/",
    "/home/",
    "/etc/",
    "/var/",
    "/private/",
    "/Volumes/",
];

impl VirtualPath {
    pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
        let normalized = normalize_absolute_path(value.into(), PathKind::Virtual)?;
        if !VIRTUAL_ROOTS
            .iter()
            .any(|root| normalized == *root || normalized.starts_with(&format!("{root}/")))
        {
            return Err(HostApiError::invalid_path(
                normalized,
                "virtual path must begin with a known root",
            ));
        }
        Ok(Self(normalized))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn join_tail(&self, tail: &str) -> Result<Self, HostApiError> {
        if tail.is_empty() {
            return Ok(self.clone());
        }
        Self::new(format!("{}/{}", self.0.trim_end_matches('/'), tail))
    }
}

impl ScopedPath {
    pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
        let raw = value.into();
        if looks_like_url(&raw) {
            return Err(HostApiError::invalid_path(raw, "URLs are not scoped paths"));
        }
        if looks_like_windows_path(&raw) {
            return Err(HostApiError::invalid_path(
                raw,
                "Windows host paths are not scoped paths",
            ));
        }
        if RAW_HOST_PREFIXES
            .iter()
            .any(|prefix| raw.starts_with(prefix))
        {
            return Err(HostApiError::invalid_path(
                raw,
                "raw host paths are not scoped paths",
            ));
        }
        let normalized = normalize_absolute_path(raw, PathKind::Scoped)?;
        Ok(Self(normalized))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl MountAlias {
    pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
        let normalized = normalize_absolute_path(value.into(), PathKind::MountAlias)?;
        Ok(Self(normalized))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy)]
enum PathKind {
    Virtual,
    Scoped,
    MountAlias,
}

fn normalize_absolute_path(raw: String, kind: PathKind) -> Result<String, HostApiError> {
    if raw.is_empty() {
        return Err(HostApiError::invalid_path(raw, "path must not be empty"));
    }
    if raw.contains('\0') || raw.chars().any(char::is_control) {
        return Err(HostApiError::invalid_path(
            raw,
            "NUL/control characters are not allowed",
        ));
    }
    if raw.contains('\\') {
        return Err(HostApiError::invalid_path(
            raw,
            "backslashes are not allowed",
        ));
    }
    if !raw.starts_with('/') {
        return Err(HostApiError::invalid_path(raw, "path must be absolute"));
    }

    let mut parts = Vec::new();
    for part in raw.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                return Err(HostApiError::invalid_path(
                    raw,
                    "`..` segments are not allowed",
                ));
            }
            part => parts.push(part),
        }
    }

    if parts.is_empty() {
        return Err(HostApiError::invalid_path(
            raw,
            "root path is not valid here",
        ));
    }

    let normalized = format!("/{}", parts.join("/"));
    if matches!(kind, PathKind::MountAlias) && normalized.ends_with('/') {
        return Err(HostApiError::invalid_path(
            normalized,
            "mount alias must not end with slash",
        ));
    }
    Ok(normalized)
}

fn looks_like_url(value: &str) -> bool {
    value.contains("://")
}

fn looks_like_windows_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3 && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/')
}
