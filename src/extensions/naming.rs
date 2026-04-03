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

pub fn legacy_extension_alias(name: &str) -> Option<String> {
    let alias = name.replace('_', "-");
    (alias != name).then_some(alias)
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
}
