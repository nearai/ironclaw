pub fn looks_like_hub_name(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut chars = s.chars();
    let first = chars.next().unwrap_or('\0');
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return false;
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

pub fn validate_hub_name(s: &str) -> anyhow::Result<()> {
    if looks_like_hub_name(s) {
        Ok(())
    } else {
        anyhow::bail!(
            "'{}' is not a valid IronHub name (lowercase letters, digits, hyphens, underscores; must start with a letter or digit).",
            s
        )
    }
}

pub fn hub_manifest_url_for_tag(tag: &str) -> anyhow::Result<String> {
    validate_release_tag(tag)?;
    Ok(format!(
        "https://github.com/nearai/ironhub/releases/download/{}/tools.json",
        tag
    ))
}

pub fn validate_release_tag(tag: &str) -> anyhow::Result<()> {
    if tag.is_empty() {
        anyhow::bail!("release tag must not be empty");
    }
    let valid = tag
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_');
    if !valid {
        anyhow::bail!(
            "release tag '{}' contains characters outside [A-Za-z0-9._-]",
            tag
        );
    }
    if tag.contains("..") {
        anyhow::bail!("release tag '{}' must not contain '..'", tag);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_like_hub_name_accepts_simple_names() {
        assert!(looks_like_hub_name("clickup"));
        assert!(looks_like_hub_name("evm-rpc"));
        assert!(looks_like_hub_name("near-rpc"));
        assert!(looks_like_hub_name("microsoft-365"));
        assert!(looks_like_hub_name("microsoft_365"));
        assert!(looks_like_hub_name("a1-b2_c3"));
        assert!(looks_like_hub_name("a"));
    }

    #[test]
    fn looks_like_hub_name_rejects_paths_and_extensions() {
        assert!(!looks_like_hub_name(""));
        assert!(!looks_like_hub_name("./local"));
        assert!(!looks_like_hub_name("/abs/path"));
        assert!(!looks_like_hub_name("tools/clickup"));
        assert!(!looks_like_hub_name("name.wasm"));
        assert!(!looks_like_hub_name("Name"));
        assert!(!looks_like_hub_name("-leading-hyphen"));
        assert!(!looks_like_hub_name("_leading-underscore"));
        assert!(!looks_like_hub_name("name with space"));
    }

    #[test]
    fn hub_manifest_url_for_tag_renders_release_url() {
        let url = hub_manifest_url_for_tag("release-2026-05-12-24").expect("valid tag");
        assert_eq!(
            url,
            "https://github.com/nearai/ironhub/releases/download/release-2026-05-12-24/tools.json"
        );
    }

    #[test]
    fn validate_release_tag_accepts_real_tags() {
        for tag in [
            "release-2026-05-12-24",
            "v1.0.0",
            "release_2026_05_12",
            "RC1",
            "0",
        ] {
            assert!(validate_release_tag(tag).is_ok(), "expected {:?}", tag);
        }
    }

    #[test]
    fn validate_release_tag_rejects_unsafe_input() {
        for tag in [
            "",
            "..",
            "v1..0",
            "../etc",
            "release/../other",
            "release@evil.com",
            "release with space",
            "release\nnewline",
            "release\0null",
            "release?query=1",
            "release#frag",
        ] {
            assert!(
                validate_release_tag(tag).is_err(),
                "expected {:?} to fail",
                tag
            );
        }
    }

    #[test]
    fn hub_manifest_url_for_tag_propagates_validation_failure() {
        let err = hub_manifest_url_for_tag("../etc").expect_err("traversal must fail");
        assert!(err.to_string().contains("characters outside"));
    }
}
