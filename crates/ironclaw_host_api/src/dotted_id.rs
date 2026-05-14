use crate::HostApiError;

pub(crate) enum VersionRule {
    Versioned,
    Unversioned,
}

pub(crate) enum PrefixRule {
    Any,
    Required(&'static str),
}

fn valid_segment_char(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
}

pub(crate) fn validate_dotted_id(
    kind: &'static str,
    value: &str,
    min_segments: usize,
    min_segments_reason: &'static str,
    prefix: PrefixRule,
    version: VersionRule,
) -> Result<(), HostApiError> {
    if value.is_empty() {
        return Err(HostApiError::invalid_id(kind, value, "must not be empty"));
    }
    if value.len() > 128 {
        return Err(HostApiError::invalid_id(
            kind,
            value,
            "must be at most 128 bytes",
        ));
    }
    if let PrefixRule::Required(required_prefix) = prefix
        && !value.starts_with(required_prefix)
    {
        return Err(HostApiError::invalid_id(
            kind,
            value,
            format!("must start with '{required_prefix}'"),
        ));
    }

    let segments = value.split('.').collect::<Vec<_>>();
    if segments.len() < min_segments {
        return Err(HostApiError::invalid_id(kind, value, min_segments_reason));
    }
    for segment in &segments {
        if segment.is_empty() {
            return Err(HostApiError::invalid_id(
                kind,
                value,
                "empty dot segments are not allowed",
            ));
        }
        if !segment.as_bytes()[0].is_ascii_lowercase() {
            return Err(HostApiError::invalid_id(
                kind,
                value,
                "segments must start with a lowercase ASCII letter",
            ));
        }
        if segment.bytes().any(|byte| !valid_segment_char(byte)) {
            return Err(HostApiError::invalid_id(
                kind,
                value,
                "only lowercase ASCII letters, digits, '_', '-', and '.' are allowed",
            ));
        }
    }

    if let VersionRule::Versioned = version {
        let version = segments[segments.len() - 1];
        let Some(rest) = version.strip_prefix('v') else {
            return Err(HostApiError::invalid_id(
                kind,
                value,
                "last segment must be a version like v1",
            ));
        };
        if rest.is_empty() || rest.bytes().any(|byte| !byte.is_ascii_digit()) {
            return Err(HostApiError::invalid_id(
                kind,
                value,
                "last segment must be a version like v1",
            ));
        }
    }

    Ok(())
}
