//! `write` engine, ported from the pinned
//! `packages/coding-agent/src/tools/write.ts` at commit
//! `08819b279cf02ae2545e69dad7111ab48d91d35e`.
//!
//! Plain files only: archives, SQLite tables/rows, `xd://` devices, and
//! `conflict://` targets are later slices. The auto-generated-file guard and
//! the read/write selector-misfire guards are not ported (later-slice
//! concerns); the hashline-prefix stripping and the exact
//! `unknown_uri_like_target` error are.

use ironclaw_filesystem::{CasExpectation, Entry, FilesystemOperation};
use serde_json::Value;

use super::hashline::format::format_hashline_header;
use super::hashline::{normalize_to_lf, strip_hashline_prefixes};
use super::state::CodingScopeKey;
use super::{
    CodingEngineContext, CodingEngineError, CodingEngineErrorKind, coding_error, display_path,
    filesystem_denied, input_error, resolve_input_path, workspace_virtual_root,
};

/// `URI_LIKE_WRITE_PATH_RE` from the pinned write.ts (`/^...$/i`).
static URI_LIKE_WRITE_PATH_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)^([a-z][a-z0-9+.-]*):/{1,2}(.*)$").expect("static uri regex") // safety: hardcoded compile-time regex literal
});
/// `XD_MISSING_DELIMITER_RE` from the pinned write.ts (`/^xd\/+(.*)$/i`).
static XD_MISSING_DELIMITER_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(
    || regex::Regex::new(r"(?i)^xd/+(.*)$").expect("static xd regex"), // safety: hardcoded compile-time regex literal
);
static LOOSE_HASHLINE_HEADER_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^\s*\[[^#\r\n]+#[^ \t\r\n]*\]\s*$").expect("static loose header regex") // safety: hardcoded compile-time regex literal
    });
/// `XD_SCHEME_NEAR_MISSES` from the pinned write.ts.
const XD_SCHEME_NEAR_MISSES: [&str; 3] = ["dx", "xdd", "xdt"];
/// Schemes the pinned internal-URL router registers handlers for. This native
/// file-only slice does not implement that router, so these targets are
/// rejected rather than becoming unreachable colon-named workspace files.
const KNOWN_INTERNAL_SCHEMES: [&str; 14] = [
    "agent", "artifact", "history", "issue", "local", "memory", "pr", "rule", "security", "skill",
    "ssh", "vault", "mcp", "xd",
];

/// Render the pinned `unknown_uri_like_target` error for the harness
/// differential seam.
pub(crate) fn render_unknown_uri_like_target(trimmed: &str, suggestion: &str) -> String {
    format!(
        "Unknown URI-like write target '{trimmed}'.{suggestion} Prefix the path with './' to write it as a filesystem path."
    )
}

/// `assertWriteTargetAddressable` from the pinned write.ts. Unknown schemes
/// and `xd/` near-misses produce the exact pinned error. Known internal
/// schemes are rejected explicitly until IronClaw's mediated internal-URL
/// router is implemented; silently treating them as file paths creates files
/// the matching read route cannot reach.
fn assert_write_target_addressable(target: &str) -> Result<(), CodingEngineError> {
    let trimmed = target.trim();
    if trimmed.starts_with('/') {
        return Ok(());
    }
    if let Some(captures) = XD_MISSING_DELIMITER_RE.captures(trimmed) {
        return Err(coding_error(
            CodingEngineErrorKind::UnknownUriLikeTarget,
            render_unknown_uri_like_target(
                trimmed,
                &format!(" Did you mean 'xd://{}'?", &captures[1]),
            ),
        ));
    }
    let Some(captures) = URI_LIKE_WRITE_PATH_RE.captures(trimmed) else {
        return Ok(());
    };
    let scheme = captures[1].to_ascii_lowercase();
    // conflict:// requires the later conflict-router slice. Reject it here
    // rather than creating an unreachable colon-named workspace file.
    if scheme == "conflict" {
        return Err(coding_error(
            CodingEngineErrorKind::UnknownUriLikeTarget,
            format!("Internal URI write target '{trimmed}' is not supported in this environment."),
        ));
    }
    if KNOWN_INTERNAL_SCHEMES.contains(&scheme.as_str()) {
        return Err(coding_error(
            CodingEngineErrorKind::UnknownUriLikeTarget,
            format!("Internal URI write target '{trimmed}' is not supported in this environment."),
        ));
    }
    let suggestion = if XD_SCHEME_NEAR_MISSES.contains(&scheme.as_str()) {
        format!(" Did you mean 'xd://{}'?", &captures[2])
    } else {
        " Tool devices use 'xd://<tool>'.".to_string()
    };
    Err(coding_error(
        CodingEngineErrorKind::UnknownUriLikeTarget,
        render_unknown_uri_like_target(trimmed, &suggestion),
    ))
}

/// `stripWriteContentWithPotentialLooseHeader` from the pinned write.ts:
/// strip a leading loose `[path#hash]` header when every remaining content
/// line is hashline-prefixed; report whether anything was stripped.
fn strip_write_content(content: &str) -> (String, bool) {
    let lines: Vec<String> = content.split('\n').map(ToString::to_string).collect();
    let header_index = lines.iter().position(|line| !line.trim().is_empty());
    let Some(header_index) = header_index else {
        return (content.to_string(), false);
    };
    if !LOOSE_HASHLINE_HEADER_RE.is_match(&lines[header_index]) {
        return (content.to_string(), false);
    }
    let mut lines_without_header: Vec<String> = Vec::with_capacity(lines.len() - 1);
    lines_without_header.extend(lines[..header_index].iter().cloned());
    lines_without_header.extend(lines[header_index + 1..].iter().cloned());
    let cleaned = strip_hashline_prefixes(&lines_without_header);
    if cleaned == lines_without_header {
        return (content.to_string(), false);
    }
    (cleaned.join("\n"), true)
}

pub(crate) async fn write(
    ctx: &CodingEngineContext,
    input: Value,
) -> Result<String, CodingEngineError> {
    let Some(path) = input.get("path").and_then(Value::as_str) else {
        return Err(input_error("write requires a string `path`"));
    };
    let Some(content) = input.get("content").and_then(Value::as_str) else {
        return Err(input_error("write requires a string `content`"));
    };

    assert_write_target_addressable(path)?;

    let (clean_content, stripped) = strip_write_content(content);

    let resolved = resolve_input_path(ctx, path, FilesystemOperation::WriteFile)?;
    match ctx.filesystem.stat(&resolved.virtual_path).await {
        Ok(stat) if stat.sensitive => return Err(filesystem_denied()),
        Ok(_) | Err(ironclaw_filesystem::FilesystemError::NotFound { .. }) => {}
        Err(error) => {
            return Err(coding_error(
                CodingEngineErrorKind::Filesystem,
                format!("filesystem error: {error}"),
            ));
        }
    }
    let display = display_path(
        &workspace_virtual_root(ctx).ok_or_else(|| {
            coding_error(
                CodingEngineErrorKind::PathResolution,
                "no workspace mount root".to_string(),
            )
        })?,
        &resolved.virtual_path,
    );

    // Write (unconditional whole-file replace; the CAS contract applies to
    // the edit engine, which reads before it writes). Parent directories
    // are established implicitly by `put` on the unified entry plane —
    // `create_dir_all` is a deprecated surface that in-memory backends do
    // not implement (mirrors the production `write_file` path, which relies
    // on the same put-established hierarchy).
    ctx.filesystem
        .put(
            &resolved.virtual_path,
            Entry::bytes(clean_content.as_bytes().to_vec()),
            CasExpectation::Any,
        )
        .await
        .map_err(|error| {
            coding_error(
                CodingEngineErrorKind::Filesystem,
                format!("filesystem error: {error}"),
            )
        })?;

    // `maybeWriteSnapshotHeader`: record the freshly-written content so
    // subsequent hashline edits address the new file with a current tag.
    let normalized = normalize_to_lf(&clean_content);
    let tag = ctx.snapshots.record_and_return(
        &CodingScopeKey::from_scope(&ctx.scope, ctx.run_id),
        resolved.virtual_path.as_str(),
        &normalized,
    );
    let header = format_hashline_header(&display, &tag);

    let write_line = format!(
        "Successfully wrote {} bytes to {display}",
        clean_content.len()
    );
    let mut result_text = format!("{header}\n{write_line}");
    if stripped {
        result_text.push_str(
            "\nNote: auto-stripped hashline display prefixes from content before writing.",
        );
    }
    Ok(result_text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_uri_like_target_messages() {
        // xd/ near-miss.
        let error = assert_write_target_addressable("xd/bar").expect_err("rejected");
        assert_eq!(
            error.message(),
            "Unknown URI-like write target 'xd/bar'. Did you mean 'xd://bar'? Prefix the path with './' to write it as a filesystem path."
        );
        // Unknown scheme -> xd hint.
        let error = assert_write_target_addressable("foo://bar").expect_err("rejected");
        assert_eq!(
            error.message(),
            "Unknown URI-like write target 'foo://bar'. Tool devices use 'xd://<tool>'. Prefix the path with './' to write it as a filesystem path."
        );
        // xd near-miss schemes suggest xd.
        let error = assert_write_target_addressable("dx://bar").expect_err("rejected");
        assert_eq!(
            error.message(),
            "Unknown URI-like write target 'dx://bar'. Did you mean 'xd://bar'? Prefix the path with './' to write it as a filesystem path."
        );
        // Known internal schemes are not file paths in this file-only slice.
        let error =
            assert_write_target_addressable("skill://my-skill/guide").expect_err("rejected");
        assert_eq!(
            error.message(),
            "Internal URI write target 'skill://my-skill/guide' is not supported in this environment."
        );
        // Plain filesystem paths pass; unrouted conflict URIs do not.
        assert!(assert_write_target_addressable("src/foo.ts").is_ok());
        assert!(assert_write_target_addressable("./foo://bar").is_ok());
        assert!(assert_write_target_addressable("conflict://2").is_err());
        // The render function keeps the same-scheme suggestion reachable.
        assert_eq!(
            render_unknown_uri_like_target("skill://x", " Did you mean 'skill://x'?"),
            "Unknown URI-like write target 'skill://x'. Did you mean 'skill://x'? Prefix the path with './' to write it as a filesystem path."
        );
    }

    #[test]
    fn strip_write_content_strips_loose_header() {
        let (cleaned, stripped) = strip_write_content("[foo.ts#1A2B]\n1:alpha\n2:beta\n");
        assert!(stripped);
        // split("\n") + join("\n") keeps the trailing empty segment, so the
        // trailing newline survives (pinned `stripWriteContentWithPotentialLooseHeader`).
        assert_eq!(cleaned, "alpha\nbeta\n");
        // Without a header, nothing is stripped.
        let (cleaned, stripped) = strip_write_content("plain content\n");
        assert!(!stripped);
        assert_eq!(cleaned, "plain content\n");
        // Non-uniform prefixes leave the content untouched.
        let (cleaned, stripped) = strip_write_content("[foo.ts#1A2B]\n1:alpha\nbeta\n");
        assert!(!stripped);
        assert_eq!(cleaned, "[foo.ts#1A2B]\n1:alpha\nbeta\n");
    }
}
