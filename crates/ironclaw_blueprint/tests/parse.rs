//! End-to-end parser tests against a full `ironclaw.config/v1` document,
//! exercising the acceptance criteria for epic #3036 slice 1. Everything
//! drives the public `parse` / `resolve_lockfile` surface — the callers the
//! CLI and admin web will use — per `.claude/rules/testing.md`.

use ironclaw_blueprint::{BlueprintError, parse, to_toml};

const FULL: &str = include_str!("fixtures/full.toml");

#[test]
fn parses_full_document() {
    let blueprint = parse(FULL).expect("full blueprint parses");
    assert_eq!(blueprint.scope.tenant.as_deref(), Some("acme"));
    assert_eq!(blueprint.extensions.len(), 1);
    assert_eq!(blueprint.extensions[0].id, "github-mcp");
    let providers = blueprint.providers.as_ref().expect("providers present");
    assert_eq!(providers.default_llm.as_deref(), Some("anthropic"));
    assert!(providers.entries.contains_key("anthropic"));
}

#[test]
fn round_trip_is_stable() {
    let first = parse(FULL).expect("parses");
    let reemitted = to_toml(&first).expect("serializes");
    let second = parse(&reemitted).expect("re-parses");
    assert_eq!(first, second, "parse -> emit -> parse must be identical");
}

#[test]
fn rejects_unknown_top_level_key() {
    let src = "api_version = \"ironclaw.config/v1\"\nkind = \"Blueprint\"\nbogus = true\n";
    let err = parse(src).expect_err("unknown key rejected");
    assert!(matches!(err, BlueprintError::Toml(_)));
}

#[test]
fn rejects_unknown_nested_key() {
    let src = "api_version = \"ironclaw.config/v1\"\nkind = \"Blueprint\"\n\
               [runtime]\nprofile = \"HostedDev\"\nmystery = 1\n";
    let err = parse(src).expect_err("unknown nested key rejected");
    assert!(matches!(err, BlueprintError::Toml(_)));
}

#[test]
fn rejects_wrong_api_version_major() {
    let src = "api_version = \"ironclaw.config/v2\"\nkind = \"Blueprint\"\n";
    let err = parse(src).expect_err("wrong major rejected");
    assert!(matches!(err, BlueprintError::UnsupportedApiVersion { .. }));
}

/// Regression: `rsplit("/v")`-based parsing accepted `ironclaw.config/v2/v1`
/// as v1 because it looked at the segment after the *last* `/v`.
#[test]
fn rejects_smuggled_api_version_major() {
    let src = "api_version = \"ironclaw.config/v2/v1\"\nkind = \"Blueprint\"\n";
    let err = parse(src).expect_err("smuggled major rejected");
    assert!(matches!(err, BlueprintError::UnsupportedApiVersion { .. }));
}

#[test]
fn accepts_minor_within_supported_major() {
    let src = "api_version = \"ironclaw.config/v1.5\"\nkind = \"Blueprint\"\n";
    parse(src).expect("minor within major accepted");
}

#[test]
fn rejects_inline_secret_pointing_at_path() {
    let src = "api_version = \"ironclaw.config/v1\"\nkind = \"Blueprint\"\n\
               [providers.anthropic]\napi_key = \"sk-proj-abcdef1234567890abcdef1234\"\n";
    let err = parse(src).expect_err("inline secret rejected");
    match err {
        BlueprintError::InlineSecret { path, .. } => {
            assert_eq!(path, "providers.anthropic.api_key");
        }
        other => panic!("expected InlineSecret, got {other:?}"),
    }
}

/// Caller-level companion to the secret-scan unit tests: a credential pasted
/// as a KEY inside the opaque `extensions[].config` table — where
/// `deny_unknown_fields` cannot see it — must still fail `parse` with the
/// offending path.
#[test]
fn rejects_inline_secret_pasted_as_config_key() {
    let src = "api_version = \"ironclaw.config/v1\"\nkind = \"Blueprint\"\n\
               [[extensions]]\nid = \"remittance\"\n\
               [extensions.config]\n\"sk-proj-abcdef1234567890abcdef1234\" = true\n";
    let err = parse(src).expect_err("secret-as-key rejected");
    match err {
        BlueprintError::InlineSecret { path, .. } => {
            assert_eq!(
                path,
                "extensions[0].config.sk-proj-abcdef1234567890abcdef1234"
            );
        }
        other => panic!("expected InlineSecret, got {other:?}"),
    }
}

#[test]
fn rejects_both_harness_id_and_inline() {
    let src = "api_version = \"ironclaw.config/v1\"\nkind = \"Blueprint\"\n\
               [harness]\nid = \"x\"\n[harness.inline]\nid = \"y\"\n";
    let err = parse(src).expect_err("ambiguous harness rejected");
    assert!(matches!(err, BlueprintError::InvalidIdentifier { .. }));
}

/// Extension/skill/provider ids use the host-api name-segment grammar:
/// lowercase-or-digit start, `a-z0-9_-.` only. `Foo` parses fine as TOML but
/// would be rejected when the apply slice constructs the typed `ExtensionId`,
/// so the parser must reject it up front.
#[test]
fn rejects_uppercase_extension_id() {
    let src = "api_version = \"ironclaw.config/v1\"\nkind = \"Blueprint\"\n\
               [[extensions]]\nid = \"Foo\"\n";
    let err = parse(src).expect_err("uppercase id rejected");
    match err {
        BlueprintError::InvalidIdentifier { path, .. } => assert_eq!(path, "extensions[0].id"),
        other => panic!("expected InvalidIdentifier, got {other:?}"),
    }
}

#[test]
fn rejects_dot_extension_id() {
    let src = "api_version = \"ironclaw.config/v1\"\nkind = \"Blueprint\"\n\
               [[extensions]]\nid = \".\"\n";
    let err = parse(src).expect_err("`.` id rejected");
    assert!(matches!(err, BlueprintError::InvalidIdentifier { .. }));
}

/// Scope ids use the host-api scope grammar (up to 256 bytes) — a 200-byte
/// tenant is valid downstream and must be valid here. (`z` rather than `a`:
/// a 200-char all-hex string would rightly trip the inline-secret LongHex
/// guard, which runs before identifier validation.)
#[test]
fn accepts_long_scope_id() {
    let tenant = "z".repeat(200);
    let src = format!(
        "api_version = \"ironclaw.config/v1\"\nkind = \"Blueprint\"\n[scope]\ntenant = \"{tenant}\"\n"
    );
    parse(&src).expect("200-byte tenant accepted");
}

#[test]
fn rejects_scope_id_with_path_separator() {
    let src = "api_version = \"ironclaw.config/v1\"\nkind = \"Blueprint\"\n\
               [scope]\ntenant = \"acme/evil\"\n";
    let err = parse(src).expect_err("separator rejected");
    assert!(matches!(err, BlueprintError::InvalidIdentifier { .. }));
}

#[test]
fn rejects_invalid_extension_version_req() {
    let src = "api_version = \"ironclaw.config/v1\"\nkind = \"Blueprint\"\n\
               [[extensions]]\nid = \"github-mcp\"\nversion = \"not a version\"\n";
    let err = parse(src).expect_err("bad version req rejected");
    match err {
        BlueprintError::InvalidVersionReq { path, .. } => {
            assert_eq!(path, "extensions[0].version");
        }
        other => panic!("expected InvalidVersionReq, got {other:?}"),
    }
}

#[test]
fn rejects_invalid_provider_table_name() {
    let src = "api_version = \"ironclaw.config/v1\"\nkind = \"Blueprint\"\n\
               [providers.\"Bad Provider\"]\nmodel = \"x\"\n";
    let err = parse(src).expect_err("bad provider name rejected");
    assert!(matches!(err, BlueprintError::InvalidIdentifier { .. }));
}

/// `applies_to` accepts the explicit `"*"` wildcard (as in the epic example)
/// or a valid scope id — anything else is rejected.
#[test]
fn applies_to_wildcard_ok_invalid_id_rejected() {
    let ok = "api_version = \"ironclaw.config/v1\"\nkind = \"Blueprint\"\n\
              [system_prompt]\ntext_ref = \"p.md\"\napplies_to = { project = \"*\" }\n";
    parse(ok).expect("wildcard accepted");

    let bad = "api_version = \"ironclaw.config/v1\"\nkind = \"Blueprint\"\n\
               [system_prompt]\ntext_ref = \"p.md\"\napplies_to = { project = \"a/b\" }\n";
    let err = parse(bad).expect_err("invalid applies_to id rejected");
    assert!(matches!(err, BlueprintError::InvalidIdentifier { .. }));
}

#[test]
fn resolves_lockfile_with_sha256() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("files/missions")).expect("mkdir");
    std::fs::write(
        dir.path().join("files/system_prompt.md"),
        b"You are Acme.\n",
    )
    .expect("write");
    std::fs::write(
        dir.path().join("files/missions/security-sweep.md"),
        b"Sweep weekly.\n",
    )
    .expect("write");

    let blueprint = parse(FULL).expect("parses");
    let lock = blueprint
        .resolve_lockfile(dir.path())
        .expect("lockfile resolves");
    assert_eq!(lock.api_version, "ironclaw.config/v1");
    assert_eq!(lock.files.len(), 2);
    // Sorted by path; every hash is 64 lowercase hex chars.
    assert_eq!(lock.files[0].path, "files/missions/security-sweep.md");
    for file in &lock.files {
        assert_eq!(file.sha256.len(), 64);
        assert!(file.sha256.chars().all(|c| c.is_ascii_hexdigit()));
    }
}

#[test]
fn lockfile_rejects_missing_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let blueprint = parse(FULL).expect("parses");
    let err = blueprint
        .resolve_lockfile(dir.path())
        .expect_err("missing file rejected");
    assert!(matches!(err, BlueprintError::FileRefRead { .. }));
}

const PROMPT_ONLY: &str = "api_version = \"ironclaw.config/v1\"\nkind = \"Blueprint\"\n\
                           [system_prompt]\ntext_ref = \"files/prompt.md\"\n";

/// The lexical `..` check cannot see symlinks; a symlink planted inside the
/// blueprint directory (e.g. arriving via a hostile GitOps repo) must not let
/// the lockfile read or hash files outside the root.
#[cfg(unix)]
#[test]
fn lockfile_rejects_symlink_escaping_root() {
    let outside = tempfile::tempdir().expect("outside dir");
    std::fs::write(outside.path().join("secret.md"), b"outside contents\n").expect("write");

    let root = tempfile::tempdir().expect("blueprint root");
    std::fs::create_dir_all(root.path().join("files")).expect("mkdir");
    std::os::unix::fs::symlink(
        outside.path().join("secret.md"),
        root.path().join("files/prompt.md"),
    )
    .expect("symlink");

    let blueprint = parse(PROMPT_ONLY).expect("parses");
    let err = blueprint
        .resolve_lockfile(root.path())
        .expect_err("escaping symlink rejected");
    assert!(
        matches!(err, BlueprintError::InvalidFileRef { .. }),
        "expected InvalidFileRef, got {err:?}"
    );
}

/// A symlink that stays inside the root is legitimate (e.g. a shared prompt
/// body linked from two refs) and must keep working.
#[cfg(unix)]
#[test]
fn lockfile_allows_symlink_within_root() {
    let root = tempfile::tempdir().expect("blueprint root");
    std::fs::create_dir_all(root.path().join("files")).expect("mkdir");
    std::fs::write(root.path().join("shared.md"), b"shared body\n").expect("write");
    std::os::unix::fs::symlink(
        root.path().join("shared.md"),
        root.path().join("files/prompt.md"),
    )
    .expect("symlink");

    let blueprint = parse(PROMPT_ONLY).expect("parses");
    let lock = blueprint
        .resolve_lockfile(root.path())
        .expect("in-root symlink ok");
    assert_eq!(lock.files.len(), 1);
    assert_eq!(lock.files[0].path, "files/prompt.md");
}
