use crate::extensions::ExtensionError;

pub fn canonicalize_extension_name(name: &str) -> Result<String, ExtensionError> {
    if name.contains('/') || name.contains('\\') || name.contains("..") || name.contains('\0') {
        return Err(ExtensionError::InstallFailed(format!(
            "Invalid extension name '{name}': contains path separator or traversal characters"
        )));
    }

    let canonical = name.trim().replace('-', "_");
    if canonical.is_empty() {
        return Err(ExtensionError::InstallFailed(
            "Invalid extension name: must not be empty".to_string(),
        ));
    }

    let bytes = canonical.as_bytes();
    if bytes.first() == Some(&b'_') || bytes.last() == Some(&b'_') {
        return Err(ExtensionError::InstallFailed(format!(
            "Invalid extension name '{name}': must start and end with a lowercase letter or digit"
        )));
    }

    let mut prev_underscore = false;
    for ch in canonical.chars() {
        let is_valid = ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_';
        if !is_valid {
            return Err(ExtensionError::InstallFailed(format!(
                "Invalid extension name '{name}': only lowercase letters, digits, and underscores are allowed"
            )));
        }
        if ch == '_' {
            if prev_underscore {
                return Err(ExtensionError::InstallFailed(format!(
                    "Invalid extension name '{name}': consecutive underscores are not allowed"
                )));
            }
            prev_underscore = true;
        } else {
            prev_underscore = false;
        }
    }

    Ok(canonical)
}

pub fn normalize_extension_names<I>(names: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    let mut normalized = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for name in names {
        match canonicalize_extension_name(&name) {
            Ok(name) => {
                if seen.insert(name.clone()) {
                    normalized.push(name);
                }
            }
            Err(error) => {
                tracing::warn!(
                    channel = %name,
                    error = %error,
                    "Skipping invalid startup channel name"
                );
            }
        }
    }

    normalized
}

pub fn legacy_extension_alias(name: &str) -> Option<String> {
    let alias = name.replace('_', "-");
    (alias != name).then_some(alias)
}

pub fn extension_name_candidates(name: &str) -> Vec<String> {
    let canonical = canonicalize_extension_name(name).unwrap_or_else(|_| name.to_string());
    let mut candidates = vec![canonical.clone()];
    if let Some(legacy) = legacy_extension_alias(&canonical) {
        candidates.push(legacy);
    }
    candidates
}

/// Filenames to look for when extracting a WASM archive for an extension.
///
/// Returns the canonical filenames (underscores) and, when the name contains
/// underscores, the pre-v0.23 hyphenated variants so that older release
/// artifacts remain installable.
pub struct ArchiveFilenames {
    pub wasm: String,
    pub caps: String,
    pub alias_wasm: Option<String>,
    pub alias_caps: Option<String>,
}

impl ArchiveFilenames {
    pub fn new(name: &str) -> Self {
        let wasm = format!("{name}.wasm");
        let caps = format!("{name}.capabilities.json");
        let alias = legacy_extension_alias(name);
        let alias_wasm = alias.as_ref().map(|a| format!("{a}.wasm"));
        let alias_caps = alias.as_ref().map(|a| format!("{a}.capabilities.json"));
        Self {
            wasm,
            caps,
            alias_wasm,
            alias_caps,
        }
    }

    pub fn is_wasm(&self, filename: &str) -> bool {
        filename == self.wasm || self.alias_wasm.as_deref().is_some_and(|a| filename == a)
    }

    pub fn is_caps(&self, filename: &str) -> bool {
        filename == self.caps || self.alias_caps.as_deref().is_some_and(|a| filename == a)
    }

    /// Error message listing all filenames that were tried.
    pub fn wasm_not_found_msg(&self) -> String {
        match &self.alias_wasm {
            Some(alias) => format!(
                "tar.gz archive does not contain '{}' or '{}'",
                self.wasm, alias
            ),
            None => format!("tar.gz archive does not contain '{}'", self.wasm),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalizes_legacy_hyphen_names() {
        assert_eq!(
            canonicalize_extension_name("web-search").unwrap(),
            "web_search"
        );
    }

    #[test]
    fn accepts_snake_case_names() {
        assert_eq!(
            canonicalize_extension_name("google_drive").unwrap(),
            "google_drive"
        );
    }

    #[test]
    fn rejects_invalid_names() {
        assert!(canonicalize_extension_name("WebSearch").is_err());
        assert!(canonicalize_extension_name("bad__name").is_err());
        assert!(canonicalize_extension_name("../bad").is_err());
    }

    #[test]
    fn normalize_extension_names_canonicalizes_deduplicates_and_skips_invalid() {
        assert_eq!(
            normalize_extension_names(vec![
                "telegram".to_string(),
                "my-channel".to_string(),
                "my_channel".to_string(),
                "BadName".to_string(),
                "../bad".to_string(),
            ]),
            vec!["telegram", "my_channel"]
        );
    }

    #[test]
    fn archive_filenames_matches_canonical() {
        let af = ArchiveFilenames::new("gmail");
        assert!(af.is_wasm("gmail.wasm"));
        assert!(af.is_caps("gmail.capabilities.json"));
        assert!(!af.is_wasm("other.wasm"));
        assert!(af.alias_wasm.is_none());
    }

    #[test]
    fn extension_name_candidates_include_legacy_alias() {
        assert_eq!(
            extension_name_candidates("google_calendar"),
            vec!["google_calendar", "google-calendar"]
        );
    }

    #[test]
    fn extension_name_candidates_canonicalize_hyphenated_name() {
        assert_eq!(
            extension_name_candidates("slack-relay"),
            vec!["slack_relay", "slack-relay"]
        );
    }

    #[test]
    fn archive_filenames_matches_hyphenated_alias() {
        let af = ArchiveFilenames::new("google_calendar");
        assert!(af.is_wasm("google_calendar.wasm"));
        assert!(af.is_wasm("google-calendar.wasm"));
        assert!(af.is_caps("google_calendar.capabilities.json"));
        assert!(af.is_caps("google-calendar.capabilities.json"));
        assert!(!af.is_wasm("other.wasm"));
    }

    #[test]
    fn wasm_not_found_msg_includes_alias() {
        let af = ArchiveFilenames::new("google_calendar");
        let msg = af.wasm_not_found_msg();
        assert!(msg.contains("google_calendar.wasm"));
        assert!(msg.contains("google-calendar.wasm"));
    }

    #[test]
    fn wasm_not_found_msg_no_alias() {
        let af = ArchiveFilenames::new("gmail");
        let msg = af.wasm_not_found_msg();
        assert!(msg.contains("gmail.wasm"));
        assert!(!msg.contains("or"));
    }
}
