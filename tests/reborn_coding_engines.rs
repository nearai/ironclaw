//! Pinned coding engines (issue #7392, second delivery slice).
//!
//! Drives the unregistered engines at
//! `ironclaw_extension_support::coding::pinned::*` against the pinned fixture
//! snapshot (`tests/fixtures/pinned_coding_contract/`) over an in-memory
//! backend:
//!
//! 1. selector parity — ALL golden/selectors.json cases through the engine
//!    parser, exact Value equality,
//! 2. read — whole-file + range selectors, hashline header with computed
//!    tag, numbered rows, elision footer, truncation notices, directory
//!    listing format, exact errors,
//! 3. write — write/read-back byte equality, parent creation, success
//!    shape, unknown_uri_like_target exact,
//! 4. edit — read→edit PUT/CUT/REM/MV, block resolution text, chained
//!    edits, stale-anchor exact messages, noop verbatim, line/range errors,
//! 5. glob — corpus patterns incl. hidden/limit; exact errors,
//! 6. grep — `*N:line` matches, context rows, skip, line-range
//!    single-file, exact errors,
//! 7. differential seam — `compare_cases` over the golden error templates
//!    with the engine's render functions.
//!
//! This top-level bin is explicitly registered in `Cargo.toml`, matching the
//! repository coverage-map rule for every new Rust test target.

mod support;

use async_trait::async_trait;
use std::sync::Arc;

use ironclaw_extension_support::coding::pinned::{
    CodingEngineContext, CodingEngineError, CodingEngineErrorKind, CodingSnapshotRegistry, harness,
};
use ironclaw_filesystem::{
    CasExpectation, Entry, FaultInjecting, FilesystemOperation, InMemoryBackend, RecordKind,
    RootFilesystem,
};
use ironclaw_host_api::artifact::{
    ArtifactAccessError, ArtifactLineRange, ArtifactReadChunk, ArtifactReadTarget,
    ArtifactSelector, ScopedArtifactReader,
};
use ironclaw_host_api::ids::{InvocationId, RunId, UserId};
use ironclaw_host_api::mount::{MountGrant, MountPermissions, MountView};
use ironclaw_host_api::path::{MountAlias, VirtualPath};
use ironclaw_host_api::resource::ResourceScope;
use serde_json::{Value, json};
use support::pinned_coding_contract::{compare_cases, selector_cases};

struct Fixture {
    filesystem: Arc<dyn RootFilesystem>,
    mounts: MountView,
    scope: ResourceScope,
    run_id: Option<RunId>,
    snapshots: Arc<CodingSnapshotRegistry>,
}

#[test]
fn registration_assets_byte_match_pinned_fixtures() {
    use ironclaw_extension_support::coding::pinned::pinned_assets;

    let cases = [
        (
            pinned_assets::CODING_WRITE_SCHEMA,
            include_str!("fixtures/pinned_coding_contract/schemas/write.json"),
        ),
        (
            pinned_assets::CODING_EDIT_SCHEMA,
            include_str!("fixtures/pinned_coding_contract/schemas/edit.json"),
        ),
        (
            pinned_assets::CODING_GLOB_SCHEMA,
            include_str!("fixtures/pinned_coding_contract/schemas/glob.json"),
        ),
        (
            pinned_assets::CODING_GREP_SCHEMA,
            include_str!("fixtures/pinned_coding_contract/schemas/grep.json"),
        ),
        (
            pinned_assets::CODING_WRITE_DESCRIPTION,
            include_str!("fixtures/pinned_coding_contract/prompts/write.md"),
        ),
        (
            pinned_assets::CODING_EDIT_DESCRIPTION,
            include_str!("fixtures/pinned_coding_contract/prompts/hashline.md"),
        ),
        (
            pinned_assets::CODING_GLOB_DESCRIPTION,
            include_str!("fixtures/pinned_coding_contract/prompts/glob.md"),
        ),
        (
            pinned_assets::CODING_GREP_DESCRIPTION,
            include_str!("fixtures/pinned_coding_contract/prompts/grep.md"),
        ),
    ];
    for (asset, fixture) in cases {
        assert_eq!(
            asset, fixture,
            "registration asset drifted from pinned fixture"
        );
    }
}

#[test]
fn registered_read_description_only_advertises_supported_sources() {
    use ironclaw_extension_support::coding::pinned::pinned_assets;

    let description = pinned_assets::CODING_READ_DESCRIPTION;
    for unsupported in [
        "SHOULD use `read` (not browser) for web content",
        "SQLite (`.sqlite`",
        "Archives (`.tar`",
        "Documents → extracted text",
        "Images → decoded inline",
        "ssh://host/<path>` reads",
    ] {
        assert!(
            !description.contains(unsupported),
            "registered read description advertises unsupported behavior {unsupported:?}"
        );
    }
    assert!(description.contains("files"));
    assert!(description.contains("directories"));
    assert!(description.contains("artifact://<id>"));
    assert!(description.contains("does not fetch web URLs"));
    assert!(description.contains("not supported by this implementation"));

    let schema: Value = serde_json::from_str(pinned_assets::CODING_READ_SCHEMA)
        .expect("registered read schema is valid JSON");
    let path_description = schema["properties"]["path"]["description"]
        .as_str()
        .expect("read path has a description");
    assert!(path_description.contains("Scoped workspace file or directory"));
    assert!(path_description.contains("artifact://<id>"));
    assert!(path_description.contains("Web URLs and other URI schemes are not supported"));
    assert!(!path_description.contains("memory://"));
}

#[test]
fn registered_bash_contract_preserves_the_supported_omp_subset() {
    use ironclaw_extension_support::coding::pinned::pinned_assets;

    let mut upstream: Value = serde_json::from_str(include_str!(
        "fixtures/pinned_coding_contract/schemas/bash.json"
    ))
    .expect("pinned upstream bash schema is valid JSON");
    let mut registered: Value = serde_json::from_str(pinned_assets::CODING_BASH_SCHEMA)
        .expect("registered bash schema is valid JSON");

    upstream["properties"]
        .as_object_mut()
        .expect("upstream bash properties")
        .remove("pty");
    assert_eq!(
        registered["properties"], upstream["properties"],
        "supported bash fields and their descriptions must stay pinned to OMP"
    );
    assert_eq!(registered["required"], upstream["required"]);
    assert_eq!(
        registered["additionalProperties"],
        upstream["additionalProperties"]
    );
    assert!(
        registered["properties"]
            .as_object_mut()
            .expect("registered bash properties")
            .keys()
            .all(|name| !matches!(name.as_str(), "pty" | "async"))
    );

    let description = pinned_assets::CODING_BASH_DESCRIPTION;
    assert!(description.contains("Runs commands in a shell."));
    assert!(description.contains("output is captured, truncated, and linked as `artifact://<id>`"));
    for unsupported in ["persistent shell", "`pty: true`", "`async: true`"] {
        assert!(
            !description.contains(unsupported),
            "registered bash description advertises unsupported behavior {unsupported:?}"
        );
    }
}

impl Fixture {
    fn new() -> Self {
        Self::with_permissions(MountPermissions::read_write_list_delete())
    }

    fn with_permissions(permissions: MountPermissions) -> Self {
        Self::with_backend(Arc::new(InMemoryBackend::new()), permissions)
    }

    /// Wrap a caller-supplied backend (e.g. an op-recording
    /// [`FaultInjecting`] decorator) instead of a bare in-memory backend.
    fn with_backend(filesystem: Arc<dyn RootFilesystem>, permissions: MountPermissions) -> Self {
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").expect("mount alias"),
            VirtualPath::new("/projects/workspace").expect("virtual path"),
            permissions,
        )])
        .expect("mount view");
        let scope = ResourceScope::local_default(
            UserId::new("pinned-harness").expect("user id"),
            InvocationId::new(),
        )
        .expect("scope");
        Self {
            filesystem,
            mounts,
            scope,
            run_id: Some(RunId::new()),
            snapshots: Arc::new(CodingSnapshotRegistry::default()),
        }
    }

    fn ctx(&self) -> CodingEngineContext {
        CodingEngineContext {
            artifact_reader: None,
            filesystem: self.filesystem.clone(),
            mounts: self.mounts.clone(),
            scope: self.scope.clone(),
            run_id: self.run_id,
            snapshots: self.snapshots.clone(),
            process: None,
        }
    }

    fn ctx_with_run(&self, run_id: Option<RunId>) -> CodingEngineContext {
        CodingEngineContext {
            artifact_reader: None,
            filesystem: self.filesystem.clone(),
            mounts: self.mounts.clone(),
            scope: self.scope.clone(),
            run_id,
            snapshots: self.snapshots.clone(),
            process: None,
        }
    }

    async fn seed(&self, path: &str, content: &str) {
        let ctx = self.ctx();
        let resolved = ctx
            .mounts
            .resolve_with_grant(&ctx.mounts.scoped_path(path).expect("scoped path"))
            .expect("grant");
        // The unified entry plane establishes parent directories implicitly
        // from the path prefix — `put` alone seeds the hierarchy (the
        // in-memory backend implements no `create_dir_all`).
        ctx.filesystem
            .put(
                &resolved.0,
                Entry::bytes(content.as_bytes().to_vec()),
                CasExpectation::Any,
            )
            .await
            .expect("seed write");
    }

    async fn seed_sensitive(&self, path: &str, content: &str) {
        let ctx = self.ctx();
        let resolved = ctx
            .mounts
            .resolve_with_grant(&ctx.mounts.scoped_path(path).expect("scoped path"))
            .expect("grant");
        let entry = Entry::record(
            RecordKind::new("sensitive_test_record").expect("record kind"),
            &json!({ "content": content }),
        )
        .expect("sensitive record");
        ctx.filesystem
            .put(&resolved.0, entry, CasExpectation::Any)
            .await
            .expect("seed sensitive entry");
    }

    /// Create a real directory (for the directory-error cases). On the
    /// prefix-inferred entry plane a directory exists once a child entry
    /// is stored under it, so seed a `.keep` marker.
    async fn seed_dir(&self, path: &str) {
        let ctx = self.ctx();
        let resolved = ctx
            .mounts
            .resolve_with_grant(&ctx.mounts.scoped_path(path).expect("scoped path"))
            .expect("grant");
        let marker = VirtualPath::new(format!(
            "{}/.keep",
            resolved.0.as_str().trim_end_matches('/')
        ))
        .expect("dir marker path");
        ctx.filesystem
            .put(&marker, Entry::bytes(Vec::new()), CasExpectation::Any)
            .await
            .expect("seed dir marker");
    }

    async fn read_back(&self, path: &str) -> String {
        let ctx = self.ctx();
        let resolved = ctx
            .mounts
            .resolve_with_grant(&ctx.mounts.scoped_path(path).expect("scoped path"))
            .expect("grant");
        let versioned = ctx
            .filesystem
            .get(&resolved.0)
            .await
            .expect("get")
            .expect("exists");
        String::from_utf8(versioned.entry.body).expect("utf8")
    }

    async fn read_tag(&self, path: &str) -> String {
        let result =
            ironclaw_extension_support::coding::pinned::read(&self.ctx(), json!({ "path": path }))
                .await
                .expect("read");
        let header = output(result)
            .split('\n')
            .next()
            .expect("header")
            .to_string();
        header
            .trim_start_matches('[')
            .split('#')
            .nth(1)
            .expect("tag")
            .trim_end_matches(']')
            .to_string()
    }
}

struct FixedArtifactReader;

#[async_trait]
impl ScopedArtifactReader for FixedArtifactReader {
    async fn read(
        &self,
        target: ArtifactReadTarget,
    ) -> Result<Option<ArtifactReadChunk>, ArtifactAccessError> {
        assert_eq!(
            target.selector,
            ArtifactSelector::Lines(ArtifactLineRange { start: 2, end: 3 })
        );
        assert_eq!(
            target.max_output_bytes,
            50 * 1024,
            "artifact line reads budget max(DEFAULT_MAX_BYTES, lines * 512) so selected lines fit"
        );
        Ok(Some(ArtifactReadChunk {
            content: b"beta\ngamma\n".to_vec(),
            content_type: "application/json".to_string(),
            total_bytes: 17,
            total_lines: Some(3),
            complete: false,
        }))
    }
}

struct ByteRangeArtifactReader;

#[async_trait]
impl ScopedArtifactReader for ByteRangeArtifactReader {
    async fn read(
        &self,
        target: ArtifactReadTarget,
    ) -> Result<Option<ArtifactReadChunk>, ArtifactAccessError> {
        assert_eq!(
            target.selector,
            ArtifactSelector::Bytes(ironclaw_host_api::artifact::ArtifactByteRange {
                start: 60_000,
                end: 60_127,
            })
        );
        assert_eq!(
            target.max_output_bytes, 128,
            "explicit artifact byte ranges are bound by their own width"
        );
        Ok(Some(ArtifactReadChunk {
            content: vec![b'x'; 128],
            content_type: "application/json".to_string(),
            total_bytes: 100_000,
            total_lines: Some(1),
            complete: false,
        }))
    }
}

fn output(value: Value) -> String {
    value
        .get("output")
        .and_then(Value::as_str)
        .expect("output text")
        .to_string()
}

fn error_message(error: &CodingEngineError) -> String {
    error.message().to_string()
}

// ─── 1. Selector parity ─────────────────────────────────────────────────────

#[test]
fn selector_parity_matches_all_golden_cases() {
    let cases = selector_cases();
    assert_eq!(cases.len(), 29, "golden selector case count");
    let mismatches = compare_cases(
        &cases,
        |_, case| case.sel.clone(),
        |case| match harness::parse_selector(&case.sel) {
            Ok(value) => Ok(value),
            Err(message) => Ok(json!({ "error": message })),
        },
        |case| {
            let mut value = json!({});
            if let Some(selector) = &case.selector {
                value["selector"] = selector.clone();
            }
            if let Some(offset_limit) = &case.offset_limit {
                value["offset_limit"] = offset_limit.clone();
            }
            if let Some(error) = &case.error {
                value["error"] = json!(error);
            }
            Ok(value)
        },
    );
    assert!(
        mismatches.is_empty(),
        "selector parity mismatches: {mismatches:#?}"
    );
}

#[test]
fn selector_overflow_inputs_fail_without_panicking() {
    assert!(harness::parse_selector("1234567890123456789012345").is_err());
    assert!(harness::parse_selector("18446744073709551615+5").is_err());

    let parsed = harness::parse_selector("1-18446744073709551615,5")
        .expect("u64::MAX endpoint merges without overflow");
    assert_eq!(
        parsed["offset_limit"],
        json!({ "offset": 1, "limit": u64::MAX })
    );
}

// ─── 2. Read ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn read_whole_file_hashline_output() {
    let fixture = Fixture::new();
    fixture
        .seed(
            "/workspace/main.rs",
            "fn main() {\n    println!(\"hi\");\n}\n",
        )
        .await;

    let result = ironclaw_extension_support::coding::pinned::read(
        &fixture.ctx(),
        json!({ "path": "main.rs" }),
    )
    .await
    .expect("read");
    let text = output(result);
    let lines: Vec<&str> = text.split('\n').collect();
    let header = lines[0];
    assert!(
        header.starts_with("[main.rs#") && header.ends_with(']'),
        "header: {header}"
    );
    assert_eq!(lines[1], "1:fn main() {");
    assert_eq!(lines[2], "2:    println!(\"hi\");");
    assert_eq!(lines[3], "3:}");
    let tag = header
        .trim_start_matches("[main.rs#")
        .trim_end_matches(']')
        .to_string();
    assert_eq!(tag.len(), 4);
    assert!(tag.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(tag, tag.to_ascii_uppercase());
}

#[tokio::test]
async fn read_range_selector_with_context() {
    let fixture = Fixture::new();
    let content: String = (1..=40).map(|n| format!("line {n}\n")).collect();
    fixture.seed("/workspace/foo.txt", &content).await;

    let result = ironclaw_extension_support::coding::pinned::read(
        &fixture.ctx(),
        json!({ "path": "foo.txt:30-35" }),
    )
    .await
    .expect("read");
    let text = output(result);
    assert!(
        text.starts_with("[foo.txt#"),
        "{}",
        text.chars().take(60).collect::<String>()
    );
    assert!(text.contains("29:line 29"), "leading context: {text}");
    assert!(text.contains("30:line 30"));
    assert!(text.contains("35:line 35"));
    assert!(text.contains("38:line 38"), "trailing context: {text}");
    assert!(!text.contains("39:line 39"), "no extra trailing context");
}

#[tokio::test]
async fn read_artifact_uri_uses_scoped_reader_and_inline_selector() {
    let fixture = Fixture::new();
    let mut context = fixture.ctx();
    context.artifact_reader = Some(Arc::new(FixedArtifactReader));

    let result = ironclaw_extension_support::coding::pinned::read(
        &context,
        json!({ "path": "artifact://7:2-3" }),
    )
    .await
    .expect("artifact selector reads");

    assert_eq!(result["output"], "2:beta\n3:gamma");
}

#[tokio::test]
async fn read_artifact_byte_selector_reaches_compact_large_results() {
    let fixture = Fixture::new();
    let mut context = fixture.ctx();
    context.artifact_reader = Some(Arc::new(ByteRangeArtifactReader));

    let result = ironclaw_extension_support::coding::pinned::read(
        &context,
        json!({ "path": "artifact://7:bytes:60000-60127" }),
    )
    .await
    .expect("artifact byte selector reads");

    assert_eq!(result["output"], "x".repeat(128));
}

struct FullArtifactReader;

#[async_trait]
impl ScopedArtifactReader for FullArtifactReader {
    async fn read(
        &self,
        target: ArtifactReadTarget,
    ) -> Result<Option<ArtifactReadChunk>, ArtifactAccessError> {
        assert_eq!(
            target.selector,
            ArtifactSelector::Lines(ArtifactLineRange {
                start: 1,
                end: 3000,
            }),
            "bare artifact reads request the default bounded line window"
        );
        assert_eq!(
            target.max_output_bytes,
            3000 * 512,
            "bare artifact reads budget the default 3000-line window (max(50 KiB, lines * 512))"
        );
        // 10 lines; the caller renders all of them and adds no elision footer.
        let content: String = (1..=10).map(|n| format!("line {n}\n")).collect();
        let byte_len = content.len() as u64;
        Ok(Some(ArtifactReadChunk {
            content: content.into_bytes(),
            content_type: "application/json".to_string(),
            total_bytes: byte_len,
            total_lines: Some(10),
            complete: true,
        }))
    }
}

struct TruncatedArtifactReader;

#[async_trait]
impl ScopedArtifactReader for TruncatedArtifactReader {
    async fn read(
        &self,
        target: ArtifactReadTarget,
    ) -> Result<Option<ArtifactReadChunk>, ArtifactAccessError> {
        assert_eq!(
            target.selector,
            ArtifactSelector::Lines(ArtifactLineRange {
                start: 1,
                end: 3000,
            })
        );
        // The reader returns only the selected 3000-line window while total
        // metadata reports two more lines for the continuation footer.
        let content = "x\n".repeat(3000);
        Ok(Some(ArtifactReadChunk {
            content: content.into_bytes(),
            content_type: "application/json".to_string(),
            total_bytes: 3002 * 2,
            total_lines: Some(3002),
            complete: false,
        }))
    }
}

/// Regression: a bare `artifact://N` read (no selector) must not fail with
/// the old 3 KiB cap — the model follows the spilled `artifact_ref` exactly.
#[tokio::test]
async fn read_artifact_bare_read_uses_3000_line_budget() {
    let fixture = Fixture::new();
    let mut context = fixture.ctx();
    context.artifact_reader = Some(Arc::new(FullArtifactReader));

    let result = ironclaw_extension_support::coding::pinned::read(
        &context,
        json!({ "path": "artifact://7" }),
    )
    .await
    .expect("bare artifact reads");

    let text = output(result.clone());
    assert!(text.starts_with("1:line 1"), "numbered lines: {text}");
    assert!(text.contains("10:line 10"));
    assert!(
        !text.contains("more lines"),
        "no footer when all lines fit: {text}"
    );
}

/// Regression (PinchBench): `grep` over a spilled artifact must search it, not
/// report the URL as a missing filesystem path.
///
/// The model learns the `artifact://N` scheme from `read` — every spilled
/// preview hands it one — and then reasonably searches inside it. All 14 such
/// grep calls in the run failed with `Path not found: artifact://N`, because the
/// engine resolved the URL against the workspace. The pinned grep accepts
/// internal URLs as search inputs (`parsePathSpecs` names `artifact://`
/// explicitly) and searches the whole resource.
#[tokio::test]
async fn grep_searches_a_spilled_artifact() {
    let fixture = Fixture::new();
    let mut context = fixture.ctx();
    context.artifact_reader = Some(Arc::new(GrepArtifactReader));

    let result = ironclaw_extension_support::coding::pinned::grep(
        &context,
        json!({ "pattern": "needle", "path": "artifact://7" }),
    )
    .await
    .expect("grep resolves an artifact URL");

    let text = output(result);
    assert!(
        text.contains("artifact://7"),
        "the artifact URL is the display path: {text}"
    );
    assert!(text.contains("needle"), "the match is reported: {text}");
    assert!(
        !text.contains("Path not found"),
        "an artifact URL is not a missing workspace path: {text}"
    );
}

/// An embedded line range is a match filter over the same resource, per the
/// pinned comment ("still honor any embedded line range as a match filter").
#[tokio::test]
async fn grep_artifact_line_range_filters_matches() {
    let fixture = Fixture::new();
    let mut context = fixture.ctx();
    context.artifact_reader = Some(Arc::new(GrepArtifactReader));

    let excluded = ironclaw_extension_support::coding::pinned::grep(
        &context,
        json!({ "pattern": "needle", "path": "artifact://7:1-3" }),
    )
    .await
    .expect("grep accepts a selector on an artifact URL");
    assert!(
        !output(excluded).contains("*5:"),
        "a range that excludes the match reports no match row"
    );

    let included = ironclaw_extension_support::coding::pinned::grep(
        &context,
        json!({ "pattern": "needle", "path": "artifact://7:4-8" }),
    )
    .await
    .expect("grep accepts a selector on an artifact URL");
    assert!(
        output(included).contains("needle"),
        "a range that covers the match reports it"
    );
}

/// Without a reader the failure must name the missing session rather than
/// claiming the path does not exist.
#[tokio::test]
async fn grep_artifact_without_a_reader_reports_no_session() {
    let fixture = Fixture::new();
    let context = fixture.ctx();

    let error = ironclaw_extension_support::coding::pinned::grep(
        &context,
        json!({ "pattern": "needle", "path": "artifact://7" }),
    )
    .await
    .expect_err("no artifact reader is configured");
    assert!(
        error_message(&error).contains("artifacts unavailable"),
        "unexpected error: {}",
        error_message(&error)
    );
}

struct GrepArtifactReader;

#[async_trait]
impl ScopedArtifactReader for GrepArtifactReader {
    async fn read(
        &self,
        target: ArtifactReadTarget,
    ) -> Result<Option<ArtifactReadChunk>, ArtifactAccessError> {
        assert_eq!(
            target.selector,
            ArtifactSelector::Full,
            "grep searches the whole resource and filters by range itself"
        );
        let content: String = (1..=8)
            .map(|n| {
                if n == 5 {
                    "needle here\n".to_string()
                } else {
                    format!("line {n}\n")
                }
            })
            .collect();
        let byte_len = content.len() as u64;
        Ok(Some(ArtifactReadChunk {
            content: content.into_bytes(),
            content_type: "text/plain".to_string(),
            total_bytes: byte_len,
            total_lines: Some(8),
            complete: true,
        }))
    }
}

/// `glob` has nothing to walk inside an immutable blob, so it must say that
/// and point at the tools that do, rather than resolving the URL against the
/// workspace and reporting a missing path.
#[tokio::test]
async fn glob_rejects_an_artifact_url_with_actionable_guidance() {
    let fixture = Fixture::new();
    let context = fixture.ctx();

    let error = ironclaw_extension_support::coding::pinned::glob(
        &context,
        json!({ "path": "artifact://7" }),
    )
    .await
    .expect_err("glob does not walk artifacts");
    let message = error_message(&error);
    assert!(
        message.contains("not supported for internal URLs"),
        "unexpected error: {message}"
    );
    assert!(
        message.contains("grep") && message.contains("read"),
        "the error must name the tools that do handle it: {message}"
    );
}

/// Regression: a bare read of an artifact with more than 3000 lines renders
/// the first 3000 and tells the model how to continue.
#[tokio::test]
async fn read_artifact_bare_read_elides_past_3000_lines() {
    let fixture = Fixture::new();
    let mut context = fixture.ctx();
    context.artifact_reader = Some(Arc::new(TruncatedArtifactReader));

    let result = ironclaw_extension_support::coding::pinned::read(
        &context,
        json!({ "path": "artifact://7" }),
    )
    .await
    .expect("bare artifact reads");

    let text = output(result.clone());
    let numbered = text
        .lines()
        .filter(|line| {
            line.chars().next().is_some_and(|c| c.is_ascii_digit()) && line.contains(':')
        })
        .count();
    assert_eq!(
        numbered, 3000,
        "exactly the default line window is rendered"
    );
    assert!(
        text.ends_with("[2 more lines in artifact. Use artifact://7:3001 to continue]"),
        "elision footer: {text}"
    );
}

struct WideLineArtifactReader;

#[async_trait]
impl ScopedArtifactReader for WideLineArtifactReader {
    async fn read(
        &self,
        target: ArtifactReadTarget,
    ) -> Result<Option<ArtifactReadChunk>, ArtifactAccessError> {
        assert_eq!(
            target.selector,
            ArtifactSelector::Lines(ArtifactLineRange { start: 1, end: 200 })
        );
        // 200 lines at ~100 B each = 20 KB; the line budget
        // max(50 KiB, 200 * 512) must admit this, the old 3 KiB cap rejected it.
        assert_eq!(
            target.max_output_bytes,
            200 * 512,
            "wide line selectors scale the budget with the line count (max(50 KiB, lines * 512))"
        );
        let content: String = (1..=200).map(|n| format!("line {n}\n")).collect();
        let byte_len = content.len() as u64;
        Ok(Some(ArtifactReadChunk {
            content: content.into_bytes(),
            content_type: "application/json".to_string(),
            total_bytes: byte_len,
            total_lines: Some(200),
            complete: false,
        }))
    }
}

/// Regression: `artifact://N:1-200` (line selector) must not hit the old
/// 3 KiB cap that rejected every line read of a large artifact.
#[tokio::test]
async fn read_artifact_wide_line_selector_gets_scaled_budget() {
    let fixture = Fixture::new();
    let mut context = fixture.ctx();
    context.artifact_reader = Some(Arc::new(WideLineArtifactReader));

    let result = ironclaw_extension_support::coding::pinned::read(
        &context,
        json!({ "path": "artifact://7:1-200" }),
    )
    .await
    .expect("wide artifact line selector reads");

    let text = output(result.clone());
    assert!(text.starts_with("1:line 1"), "numbered lines: {text}");
    assert!(text.contains("200:line 200"));
}

struct OpenEndedArtifactReader;

#[async_trait]
impl ScopedArtifactReader for OpenEndedArtifactReader {
    async fn read(
        &self,
        target: ArtifactReadTarget,
    ) -> Result<Option<ArtifactReadChunk>, ArtifactAccessError> {
        assert_eq!(
            target.selector,
            ArtifactSelector::Lines(ArtifactLineRange {
                start: 50,
                end: 3049,
            }),
            "open-ended selectors are bounded to the default 3000-line window"
        );
        assert_eq!(target.max_output_bytes, 3000 * 512);
        Ok(Some(ArtifactReadChunk {
            content: b"line 50\n".to_vec(),
            content_type: "text/plain".to_string(),
            total_bytes: 8,
            total_lines: Some(50),
            complete: false,
        }))
    }
}

#[tokio::test]
async fn read_artifact_open_ended_selector_is_bounded() {
    let fixture = Fixture::new();
    let mut context = fixture.ctx();
    context.artifact_reader = Some(Arc::new(OpenEndedArtifactReader));

    ironclaw_extension_support::coding::pinned::read(
        &context,
        json!({ "path": "artifact://7:50-" }),
    )
    .await
    .expect("open-ended artifact selector reads");
}

struct OversizedRawArtifactReader;

#[async_trait]
impl ScopedArtifactReader for OversizedRawArtifactReader {
    async fn read(
        &self,
        target: ArtifactReadTarget,
    ) -> Result<Option<ArtifactReadChunk>, ArtifactAccessError> {
        assert_eq!(target.selector, ArtifactSelector::Full);
        assert_eq!(
            target.max_output_bytes,
            50 * 1024,
            "unbounded raw reads retain the upstream 50 KiB guard"
        );
        Err(ArtifactAccessError::OversizedUnsliced)
    }
}

#[tokio::test]
async fn read_artifact_rejects_oversized_unbounded_raw_reads() {
    let fixture = Fixture::new();
    let mut context = fixture.ctx();
    context.artifact_reader = Some(Arc::new(OversizedRawArtifactReader));

    let error = ironclaw_extension_support::coding::pinned::read(
        &context,
        json!({ "path": "artifact://7:raw" }),
    )
    .await
    .expect_err("oversized unbounded raw read must be rejected");

    let message = error_message(&error);
    assert!(message.contains("Unbounded raw read blocked"), "{message}");
    assert!(message.contains("artifact://7:raw:1-3000"), "{message}");
}

#[tokio::test]
async fn read_multi_range_elision_footer() {
    let fixture = Fixture::new();
    let content: String = (1..=40).map(|n| format!("line {n}\n")).collect();
    fixture.seed("/workspace/foo.txt", &content).await;

    let result = ironclaw_extension_support::coding::pinned::read(
        &fixture.ctx(),
        json!({ "path": "foo.txt:5-8,20-22" }),
    )
    .await
    .expect("read");
    let text = output(result);
    assert!(text.contains("5:line 5"));
    assert!(text.contains("8:line 8"));
    assert!(text.contains("\n…\n"), "elision separator: {text}");
    assert!(text.contains("20:line 20"));
    // Line counting mirrors the pinned multi-range reader
    // (`buildInMemoryMultiRangeResult`: `text.split("\n").length`), so the
    // trailing empty entry of newline-terminated content is line 41.
    // Visible 5-8 + 20-22 (7 of 41 lines) -> 34 elided across 3 spans
    // (1-4, 9-19, 23-41); the footer samples the FIRST two ranges with the
    // ", e.g." tail (pinned `formatSummaryElisionFooter`, FOOTER_RANGE_SAMPLES
    // = 2, verified against the pinned read-format.ts at 08819b2).
    assert!(
        text.contains("[…34ln elided; re-read needed ranges, e.g. foo.txt:1-4,9-19]"),
        "elision footer: {text}"
    );
}

#[tokio::test]
async fn read_truncation_notice_at_3000_lines() {
    let fixture = Fixture::new();
    let content: String = (1..=4000).map(|n| format!("line {n}\n")).collect();
    fixture.seed("/workspace/big.txt", &content).await;

    let result = ironclaw_extension_support::coding::pinned::read(
        &fixture.ctx(),
        json!({ "path": "big.txt" }),
    )
    .await
    .expect("read");
    let text = output(result);
    assert!(text.contains("3000:line 3000"));
    // Pinned `truncateHead` counts `countNewlines(content) + 1`, so the
    // trailing newline of the 4000th line makes the total 4001 (verified
    // against the pinned streaming-output.ts at 08819b2).
    let tail: String = text
        .chars()
        .rev()
        .take(140)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    assert!(
        text.contains("[Showing lines 1-3000 of 4001. Use :3001 to continue]"),
        "truncation notice: {tail}"
    );
    assert!(!text.contains("3001:line 3001"));
}

#[tokio::test]
async fn read_directory_listing_format() {
    let fixture = Fixture::new();
    fixture
        .seed("/workspace/src/main.rs", "fn main() {}\n")
        .await;
    fixture
        .seed("/workspace/src/lib.rs", "pub fn x() {}\n")
        .await;
    fixture.seed("/workspace/README.md", "# hi\n").await;

    let result =
        ironclaw_extension_support::coding::pinned::read(&fixture.ctx(), json!({ "path": "." }))
            .await
            .expect("read");
    let text = output(result);
    let lines: Vec<&str> = text.split('\n').collect();
    assert_eq!(lines[0], ".");
    assert!(
        lines.iter().any(|line| line.contains("- README.md")),
        "{text}"
    );
    assert!(lines.iter().any(|line| line.contains("- src/")), "{text}");
    assert!(
        lines.iter().any(|line| line.contains("- main.rs")),
        "{text}"
    );
    assert!(lines.iter().any(|line| line.contains("- lib.rs")), "{text}");
}

#[tokio::test]
async fn read_directory_stats_each_entry_once() {
    // The directory render must stat each listed entry exactly once: the
    // frontier walk carries its `FileStat` into the bucketing pass instead
    // of re-stat'ing every collected entry for `modified`/`len`. Record the
    // genuine backend traffic through the shared `FaultInjecting` decorator.
    let backend = Arc::new(FaultInjecting::new(InMemoryBackend::new()));
    let fixture =
        Fixture::with_backend(backend.clone(), MountPermissions::read_write_list_delete());
    fixture.seed("/workspace/a.txt", "a\n").await;
    fixture.seed("/workspace/b.txt", "b\n").await;
    fixture.seed("/workspace/sub/c.txt", "c\n").await;

    let result =
        ironclaw_extension_support::coding::pinned::read(&fixture.ctx(), json!({ "path": "." }))
            .await
            .expect("read");
    let text = output(result);
    for expected in ["- a.txt", "- b.txt", "- sub/", "- c.txt"] {
        assert!(text.contains(expected), "{expected} missing from: {text}");
    }

    // One stat per collected entry (a.txt, b.txt, sub, c.txt) plus the
    // single is-directory probe on the root; a second bucketing stat per
    // entry would record 9 instead of 5.
    assert_eq!(backend.count(FilesystemOperation::Stat), 5);
}

#[tokio::test]
async fn read_errors_exact() {
    let fixture = Fixture::new();
    fixture.seed_dir("/workspace/dir").await;

    let error = ironclaw_extension_support::coding::pinned::read(
        &fixture.ctx(),
        json!({ "path": "gone.rs" }),
    )
    .await
    .expect_err("not found");
    assert_eq!(error_message(&error), "Path 'gone.rs' not found");
    assert_eq!(error.kind(), CodingEngineErrorKind::PathNotFound);

    let error = ironclaw_extension_support::coding::pinned::read(
        &fixture.ctx(),
        json!({ "path": "dir:1-5,10-20" }),
    )
    .await
    .expect_err("multi-range directory");
    assert_eq!(
        error_message(&error),
        "Multi-range line selectors are not supported for directory listings."
    );

    // `foo.txt:raw:raw` splits strictly to path `foo.txt:raw` + selector
    // `raw` (the strict splitter only peels one selector-shaped tail), so
    // the missing path error is the pinned outcome — the `raw:raw`
    // invalid-selector text belongs to the parse level (selectors.json).
    let error = ironclaw_extension_support::coding::pinned::read(
        &fixture.ctx(),
        json!({ "path": "foo.txt:raw:raw" }),
    )
    .await
    .expect_err("peeled raw tail missing");
    assert_eq!(error_message(&error), "Path 'foo.txt:raw' not found");
    assert_eq!(error.kind(), CodingEngineErrorKind::PathNotFound);
}

#[tokio::test]
async fn read_rejects_metadata_sensitive_and_oversized_files() {
    let fixture = Fixture::new();
    fixture
        .seed_sensitive("/workspace/opaque-name.json", "do not expose")
        .await;

    let error = ironclaw_extension_support::coding::pinned::read(
        &fixture.ctx(),
        json!({ "path": "opaque-name.json" }),
    )
    .await
    .expect_err("metadata-sensitive entry is denied");
    assert_eq!(error_message(&error), "workspace file access denied");

    let oversized = "x".repeat(10 * 1024 * 1024 + 1);
    fixture.seed("/workspace/oversized.txt", &oversized).await;
    let error = ironclaw_extension_support::coding::pinned::read(
        &fixture.ctx(),
        json!({ "path": "oversized.txt" }),
    )
    .await
    .expect_err("oversized file rejected before materialization");
    assert_eq!(
        error_message(&error),
        "workspace file exceeds the read limit"
    );
}

// ─── 3. Write ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn write_then_read_back_byte_equality_and_parents() {
    let fixture = Fixture::new();
    let result = ironclaw_extension_support::coding::pinned::write(
        &fixture.ctx(),
        json!({ "path": "src/nested/new.txt", "content": "hello\nworld\n" }),
    )
    .await
    .expect("write");
    let text = output(result);
    assert!(
        text.contains("Successfully wrote 12 bytes to src/nested/new.txt"),
        "{text}"
    );
    assert_eq!(
        fixture.read_back("/workspace/src/nested/new.txt").await,
        "hello\nworld\n"
    );
}

#[tokio::test]
async fn write_success_shape_has_snapshot_header() {
    let fixture = Fixture::new();
    let result = ironclaw_extension_support::coding::pinned::write(
        &fixture.ctx(),
        json!({ "path": "a.txt", "content": "abc" }),
    )
    .await
    .expect("write");
    let text = output(result);
    let first = text.split('\n').next().expect("first line");
    assert!(
        first.starts_with("[a.txt#") && first.ends_with(']'),
        "{first}"
    );
    assert!(
        text.ends_with("Successfully wrote 3 bytes to a.txt"),
        "{text}"
    );
}

#[tokio::test]
async fn write_unknown_uri_like_target_exact() {
    let fixture = Fixture::new();
    let error = ironclaw_extension_support::coding::pinned::write(
        &fixture.ctx(),
        json!({ "path": "foo://bar", "content": "x" }),
    )
    .await
    .expect_err("uri-like rejected");
    assert_eq!(
        error_message(&error),
        "Unknown URI-like write target 'foo://bar'. Tool devices use 'xd://<tool>'. Prefix the path with './' to write it as a filesystem path."
    );
    assert_eq!(error.kind(), CodingEngineErrorKind::UnknownUriLikeTarget);
}

#[tokio::test]
async fn write_strips_hashline_prefixes() {
    let fixture = Fixture::new();
    let result = ironclaw_extension_support::coding::pinned::write(
        &fixture.ctx(),
        json!({ "path": "b.txt", "content": "[b.txt#1A2B]\n1:alpha\n2:beta\n" }),
    )
    .await
    .expect("write");
    let text = output(result);
    assert!(
        text.ends_with(
            "Note: auto-stripped hashline display prefixes from content before writing."
        ),
        "{text}"
    );
    // Pinned `stripWriteContentWithPotentialLooseHeader` splits on `\n` and
    // joins with `\n`, so the trailing empty segment of the split survives:
    // the stripped content keeps its trailing newline (verified against the
    // pinned write.ts at 08819b2).
    assert_eq!(fixture.read_back("/workspace/b.txt").await, "alpha\nbeta\n");
}

#[tokio::test]
async fn write_rejects_metadata_sensitive_existing_file() {
    let fixture = Fixture::new();
    fixture
        .seed_sensitive("/workspace/opaque-name.json", "do not overwrite")
        .await;

    let error = ironclaw_extension_support::coding::pinned::write(
        &fixture.ctx(),
        json!({ "path": "opaque-name.json", "content": "replacement" }),
    )
    .await
    .expect_err("metadata-sensitive entry is denied");
    assert_eq!(error_message(&error), "workspace file access denied");
}

// ─── 4. Edit ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn edit_put_single_line_success_and_chained_edits() {
    let fixture = Fixture::new();
    fixture
        .seed("/workspace/foo.ts", "line1\nline2\nline3\n")
        .await;
    let tag = fixture.read_tag("foo.ts").await;

    let result = ironclaw_extension_support::coding::pinned::edit(
        &fixture.ctx(),
        json!({ "input": format!("[foo.ts#{tag}]\nPUT 2:\n+CHANGED\n") }),
    )
    .await
    .expect("edit");
    let text = output(result);
    assert!(text.starts_with("[foo.ts#"), "header: {text}");
    assert!(
        text.contains("2:CHANGED"),
        "preview shows the new line: {text}"
    );
    assert_eq!(
        fixture.read_back("/workspace/foo.ts").await,
        "line1\nCHANGED\nline3\n"
    );

    // Chained edit with the refreshed tag works without a re-read.
    let new_tag = text
        .split('\n')
        .next()
        .expect("header")
        .trim_start_matches('[')
        .split('#')
        .nth(1)
        .expect("tag")
        .trim_end_matches(']')
        .to_string();
    let result = ironclaw_extension_support::coding::pinned::edit(
        &fixture.ctx(),
        json!({ "input": format!("[foo.ts#{new_tag}]\nPUT 3:\n+line4\n") }),
    )
    .await
    .expect("chained edit");
    let chained_text = output(result);
    assert!(chained_text.contains("3:line4"), "{}", chained_text);
    assert_eq!(
        fixture.read_back("/workspace/foo.ts").await,
        "line1\nCHANGED\nline4\n"
    );
}

#[tokio::test]
async fn edit_cut_rem_mv() {
    let fixture = Fixture::new();
    fixture.seed("/workspace/cut.ts", "a\nb\nc\n").await;
    let tag = fixture.read_tag("cut.ts").await;
    let result = ironclaw_extension_support::coding::pinned::edit(
        &fixture.ctx(),
        json!({ "input": format!("[cut.ts#{tag}]\nCUT 2\n") }),
    )
    .await
    .expect("cut");
    assert!(output(result).starts_with("[cut.ts#"));
    assert_eq!(fixture.read_back("/workspace/cut.ts").await, "a\nc\n");

    fixture.seed("/workspace/old.ts", "x\n").await;
    let tag = fixture.read_tag("old.ts").await;
    let result = ironclaw_extension_support::coding::pinned::edit(
        &fixture.ctx(),
        json!({ "input": format!("[old.ts#{tag}]\nMV renamed.ts\n") }),
    )
    .await
    .expect("mv");
    let text = output(result);
    assert!(text.contains("Moved to renamed.ts"), "{text}");
    assert_eq!(fixture.read_back("/workspace/renamed.ts").await, "x\n");

    let tag = fixture.read_tag("renamed.ts").await;
    let result = ironclaw_extension_support::coding::pinned::edit(
        &fixture.ctx(),
        json!({ "input": format!("[renamed.ts#{tag}]\nREM\n") }),
    )
    .await
    .expect("rem");
    assert_eq!(output(result), "Deleted renamed.ts");
    let error = ironclaw_extension_support::coding::pinned::read(
        &fixture.ctx(),
        json!({ "path": "renamed.ts" }),
    )
    .await
    .expect_err("gone");
    assert_eq!(error_message(&error), "Path 'renamed.ts' not found");
}

#[tokio::test]
async fn edit_block_resolution_text() {
    let fixture = Fixture::new();
    fixture
        .seed("/workspace/block.ts", "fn main() {\n    let x = 1;\n}\n")
        .await;
    let tag = fixture.read_tag("block.ts").await;
    let result = ironclaw_extension_support::coding::pinned::edit(
        &fixture.ctx(),
        json!({ "input": format!("[block.ts#{tag}]\nPUT 1*:\n+fn main() {{\n+    let x = 2;\n+}}\n") }),
    )
    .await
    .expect("block edit");
    let text = output(result);
    assert!(
        text.contains("PUT 1*: → resolved lines 1-3 (3 lines)"),
        "block resolution text: {text}"
    );
    assert_eq!(
        fixture.read_back("/workspace/block.ts").await,
        "fn main() {\n    let x = 2;\n}\n"
    );
}

#[tokio::test]
async fn edit_stale_anchor_recognized_exact_and_never_writes() {
    let fixture = Fixture::new();
    fixture.seed("/workspace/foo.ts", "line1\nline2\n").await;
    let tag = fixture.read_tag("foo.ts").await;
    // The file changes between read and edit (external write).
    fixture
        .seed("/workspace/foo.ts", "line1\nEDITED\nline3\n")
        .await;

    let error = ironclaw_extension_support::coding::pinned::edit(
        &fixture.ctx(),
        json!({ "input": format!("[foo.ts#{tag}]\nPUT 2:\n+new\n") }),
    )
    .await
    .expect_err("stale anchor");
    let message = error_message(&error);
    assert!(
        message.starts_with("Edit rejected for foo.ts: file changed between read and edit."),
        "{message}"
    );
    assert!(
        message.contains(&format!(
            "Section is bound to #{tag}, but the current file hashes to #"
        )),
        "{message}"
    );
    assert!(message.contains("re-read the file with `read` to refresh the tag before retrying."));
    assert_eq!(
        error.kind(),
        CodingEngineErrorKind::StaleAnchorHashRecognized
    );
    // The stale-anchor edit must never perform a (fuzzy) write.
    assert_eq!(
        fixture.read_back("/workspace/foo.ts").await,
        "line1\nEDITED\nline3\n"
    );
}

#[tokio::test]
async fn edit_rejects_changed_content_that_collides_on_the_display_tag() {
    let fixture = Fixture::new();
    let mut by_tag = std::collections::HashMap::<String, String>::new();
    let (before, collided, tag) = (0u64..100_000)
        .find_map(|n| {
            let candidate = format!("value-{n}\n");
            let tag = harness::compute_file_hash(&candidate);
            by_tag
                .insert(tag.clone(), candidate.clone())
                .filter(|prior| prior != &candidate)
                .map(|prior| (prior, candidate, tag))
        })
        .expect("16-bit display tags produce a deterministic collision");

    fixture.seed("/workspace/collision.txt", &before).await;
    assert_eq!(fixture.read_tag("collision.txt").await, tag);
    fixture.seed("/workspace/collision.txt", &collided).await;

    let error = ironclaw_extension_support::coding::pinned::edit(
        &fixture.ctx(),
        json!({ "input": format!("[collision.txt#{tag}]\nPUT 1:\n+replacement\n") }),
    )
    .await
    .expect_err("full snapshot fingerprint detects the changed file");
    assert_eq!(
        error.kind(),
        CodingEngineErrorKind::StaleAnchorHashRecognized
    );
    assert_eq!(
        fixture.read_back("/workspace/collision.txt").await,
        collided
    );
}

#[tokio::test]
async fn edit_rem_and_move_require_delete_permission_before_writing() {
    let fixture = Fixture::with_permissions(MountPermissions::read_write());
    fixture.seed("/workspace/source.txt", "original\n").await;
    let tag = fixture.read_tag("source.txt").await;

    let rem_error = ironclaw_extension_support::coding::pinned::edit(
        &fixture.ctx(),
        json!({ "input": format!("[source.txt#{tag}]\nREM\n") }),
    )
    .await
    .expect_err("REM requires delete permission");
    assert_eq!(error_message(&rem_error), "workspace file access denied");
    assert_eq!(
        fixture.read_back("/workspace/source.txt").await,
        "original\n"
    );

    let move_error = ironclaw_extension_support::coding::pinned::edit(
        &fixture.ctx(),
        json!({ "input": format!("[source.txt#{tag}]\nMV destination.txt\n") }),
    )
    .await
    .expect_err("MV checks delete permission before writing the destination");
    assert_eq!(error_message(&move_error), "workspace file access denied");
    assert_eq!(
        fixture.read_back("/workspace/source.txt").await,
        "original\n"
    );
    let destination = ironclaw_extension_support::coding::pinned::read(
        &fixture.ctx(),
        json!({ "path": "destination.txt" }),
    )
    .await;
    assert!(destination.is_err(), "destination must not be written");
}

#[tokio::test]
async fn edit_without_read_not_from_session_exact() {
    let fixture = Fixture::new();
    fixture.seed("/workspace/foo.ts", "line1\nline2\n").await;

    let error = ironclaw_extension_support::coding::pinned::edit(
        &fixture.ctx(),
        json!({ "input": "[foo.ts#9F00]\nPUT 2:\n+new\n" }),
    )
    .await
    .expect_err("unknown hash");
    let message = error_message(&error);
    assert!(
        message.starts_with("Edit rejected for foo.ts: hash #9F00 is not from this session."),
        "{message}"
    );
    assert!(message.contains("never invent the tag and never reuse one from a prior session."));
    assert_eq!(
        error.kind(),
        CodingEngineErrorKind::StaleAnchorHashUnrecognized
    );
}

#[tokio::test]
async fn edit_noop_verbatim() {
    let fixture = Fixture::new();
    fixture.seed("/workspace/foo.ts", "line1\nline2\n").await;
    let tag = fixture.read_tag("foo.ts").await;
    let result = ironclaw_extension_support::coding::pinned::edit(
        &fixture.ctx(),
        json!({ "input": format!("[foo.ts#{tag}]\nPUT 1:\n+line1\n") }),
    )
    .await
    .expect("noop");
    assert_eq!(
        output(result),
        "Edits to foo.ts parsed and applied cleanly, but produced no change: your body row(s) are byte-identical to the file at the targeted lines. The bug is somewhere else — re-read the file before issuing another edit. Do NOT widen the payload or add lines; verify the anchor first."
    );
}

#[tokio::test]
async fn edit_line_out_of_bounds_and_invalid_range() {
    let fixture = Fixture::new();
    fixture.seed("/workspace/foo.ts", "a\nb\n").await;
    let tag = fixture.read_tag("foo.ts").await;

    let error = ironclaw_extension_support::coding::pinned::edit(
        &fixture.ctx(),
        json!({ "input": format!("[foo.ts#{tag}]\nPUT 99:\n+x\n") }),
    )
    .await
    .expect_err("out of bounds");
    // `split("\n")` on newline-terminated content yields a trailing empty
    // entry; the pinned bounds message counts it (file has 3 lines).
    assert_eq!(
        error_message(&error),
        "Line 99 does not exist (file has 3 lines)"
    );
    assert_eq!(error.kind(), CodingEngineErrorKind::LineOutOfBounds);

    let error = ironclaw_extension_support::coding::pinned::edit(
        &fixture.ctx(),
        json!({ "input": format!("[foo.ts#{tag}]\nPUT 10.=5:\n+x\n") }),
    )
    .await
    .expect_err("invalid absolute range");
    assert!(
        error_message(&error).starts_with(
            "line 1: Invalid absolute range: start 10, end 5. The value after `.=` is an absolute source line, not a line count or replacement length. For one line use `PUT 10:`."
        ),
        "{}",
        error_message(&error)
    );
    assert_eq!(error.kind(), CodingEngineErrorKind::InvalidAbsoluteRange);
}

// ─── 5. Glob ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn glob_patterns_hidden_and_limit() {
    let fixture = Fixture::new();
    fixture.seed("/workspace/src/a.ts", "a\n").await;
    fixture.seed("/workspace/src/b.ts", "b\n").await;
    fixture.seed("/workspace/src/c.rs", "c\n").await;
    fixture.seed("/workspace/src/.hidden.ts", "h\n").await;

    let result = ironclaw_extension_support::coding::pinned::glob(
        &fixture.ctx(),
        json!({ "path": "src/*.ts" }),
    )
    .await
    .expect("glob");
    let text = output(result);
    assert!(text.contains("a.ts") && text.contains("b.ts"), "{text}");
    assert!(
        text.contains(".hidden.ts"),
        "hidden defaults to true: {text}"
    );
    assert!(!text.contains("c.rs"), "{text}");

    let result = ironclaw_extension_support::coding::pinned::glob(
        &fixture.ctx(),
        json!({ "path": "src/*.ts", "hidden": false, "limit": 1 }),
    )
    .await
    .expect("glob");
    let text = output(result);
    assert!(!text.contains(".hidden.ts"), "{text}");
    assert!(text.lines().count() <= 2, "limit applied: {text}");
}

#[tokio::test]
async fn glob_and_grep_filter_metadata_sensitive_entries() {
    let fixture = Fixture::new();
    fixture.seed("/workspace/public.txt", "needle\n").await;
    fixture
        .seed_sensitive("/workspace/opaque-name.json", "needle")
        .await;

    let glob =
        ironclaw_extension_support::coding::pinned::glob(&fixture.ctx(), json!({ "path": "**/*" }))
            .await
            .expect("glob");
    let glob_text = output(glob);
    assert!(glob_text.contains("public.txt"));
    assert!(!glob_text.contains("opaque-name.json"));

    let grep = ironclaw_extension_support::coding::pinned::grep(
        &fixture.ctx(),
        json!({ "pattern": "needle" }),
    )
    .await
    .expect("grep");
    let grep_text = output(grep);
    assert!(grep_text.contains("public.txt"));
    assert!(!grep_text.contains("opaque-name.json"));
}

#[tokio::test]
async fn glob_errors_exact() {
    let fixture = Fixture::new();
    let error =
        ironclaw_extension_support::coding::pinned::glob(&fixture.ctx(), json!({ "path": "/" }))
            .await
            .expect_err("root");
    assert_eq!(
        error_message(&error),
        "Searching from root directory '/' is not allowed"
    );
    assert_eq!(error.kind(), CodingEngineErrorKind::RootNotAllowed);

    let error =
        ironclaw_extension_support::coding::pinned::glob(&fixture.ctx(), json!({ "path": "" }))
            .await
            .expect_err("empty");
    assert_eq!(
        error_message(&error),
        "`path` must contain non-empty globs or paths"
    );

    let error = ironclaw_extension_support::coding::pinned::glob(
        &fixture.ctx(),
        json!({ "path": "nope.ts" }),
    )
    .await
    .expect_err("missing");
    assert_eq!(error_message(&error), "Path not found: nope.ts");
}

// ─── 6. Grep ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn grep_matches_context_and_skip() {
    let fixture = Fixture::new();
    fixture
        .seed("/workspace/src/a.ts", "one\nmatch-a\nthree\nfour\n")
        .await;
    fixture
        .seed("/workspace/src/b.ts", "one\nmatch-b\ntwo\n")
        .await;

    let result = ironclaw_extension_support::coding::pinned::grep(
        &fixture.ctx(),
        json!({ "pattern": "match" }),
    )
    .await
    .expect("grep");
    let text = output(result);
    assert!(
        text.contains("a.ts") && text.contains("b.ts"),
        "file headers: {text}"
    );
    assert!(text.contains("*2:match-a"), "match row: {text}");
    assert!(text.contains(" 1:one"), "context row before: {text}");
    assert!(text.contains(" 3:three"), "context row after: {text}");
    assert!(text.contains("*2:match-b"), "second file's match: {text}");

    let result = ironclaw_extension_support::coding::pinned::grep(
        &fixture.ctx(),
        json!({ "pattern": "match", "skip": 1 }),
    )
    .await
    .expect("grep skip");
    let text = output(result);
    assert!(
        !text.contains("match-a"),
        "skip paginates past the first file: {text}"
    );
    assert!(text.contains("match-b"), "second page has the rest: {text}");
}

#[tokio::test]
async fn grep_line_range_single_file() {
    let fixture = Fixture::new();
    let content: String = (1..=20).map(|n| format!("line {n}\n")).collect();
    fixture.seed("/workspace/target.txt", &content).await;

    let result = ironclaw_extension_support::coding::pinned::grep(
        &fixture.ctx(),
        json!({ "pattern": "line", "path": "target.txt:5-8" }),
    )
    .await
    .expect("grep");
    let text = output(result);
    assert!(text.contains("*5:line 5"), "{text}");
    assert!(text.contains("8:line 8"), "{text}");
    assert!(!text.contains("line 9"), "{text}");
}

#[tokio::test]
async fn grep_errors_exact() {
    let fixture = Fixture::new();
    fixture.seed_dir("/workspace/dir").await;

    let error =
        ironclaw_extension_support::coding::pinned::grep(&fixture.ctx(), json!({ "pattern": "(" }))
            .await
            .expect_err("invalid regex");
    assert!(
        error_message(&error).starts_with("Invalid regex: "),
        "{}",
        error_message(&error)
    );
    assert_eq!(error.kind(), CodingEngineErrorKind::InvalidRegex);

    let error = ironclaw_extension_support::coding::pinned::grep(
        &fixture.ctx(),
        json!({ "pattern": "  " }),
    )
    .await
    .expect_err("empty pattern");
    assert_eq!(error_message(&error), "Pattern must not be empty");

    let error = ironclaw_extension_support::coding::pinned::grep(
        &fixture.ctx(),
        json!({ "pattern": "x", "skip": -1 }),
    )
    .await
    .expect_err("negative skip");
    assert_eq!(error_message(&error), "Skip must be a non-negative number");

    let error = ironclaw_extension_support::coding::pinned::grep(
        &fixture.ctx(),
        json!({ "pattern": "x", "path": "src/*.ts:50-100" }),
    )
    .await
    .expect_err("range on glob");
    assert_eq!(
        error_message(&error),
        "Line-range selector requires a single file, not a glob: src/*.ts:50-100"
    );

    let error = ironclaw_extension_support::coding::pinned::grep(
        &fixture.ctx(),
        json!({ "pattern": "x", "path": "dir:1-5" }),
    )
    .await
    .expect_err("range on dir");
    assert_eq!(
        error_message(&error),
        "Line-range selector requires a single file: dir:1-5 is a directory"
    );

    let error = ironclaw_extension_support::coding::pinned::grep(
        &fixture.ctx(),
        json!({ "pattern": "x", "path": "gone.rs:1-5" }),
    )
    .await
    .expect_err("range path missing");
    assert_eq!(
        error_message(&error),
        "Path not found for line-range selector: gone.rs:1-5"
    );
}

#[tokio::test]
async fn grep_case_option_polarity() {
    let fixture = Fixture::new();
    fixture
        .seed("/workspace/case.txt", "needle\nNEEDLE\nNeedle\n")
        .await;

    // Omitted `case` defaults to case-sensitive (pinned schema: `case`
    // is "case-sensitive search").
    let result = ironclaw_extension_support::coding::pinned::grep(
        &fixture.ctx(),
        json!({ "pattern": "needle" }),
    )
    .await
    .expect("grep default case");
    let text = output(result);
    assert!(text.contains("*1:needle"), "default case-sensitive: {text}");
    assert!(
        !text.contains("*2:NEEDLE"),
        "default case-sensitive: {text}"
    );

    // `case: true` stays case-sensitive.
    let result = ironclaw_extension_support::coding::pinned::grep(
        &fixture.ctx(),
        json!({ "pattern": "needle", "case": true }),
    )
    .await
    .expect("grep case true");
    let text = output(result);
    assert!(text.contains("*1:needle"), "case-sensitive: {text}");
    assert!(!text.contains("*2:NEEDLE"), "case-sensitive: {text}");

    // `case: false` is case-insensitive.
    let result = ironclaw_extension_support::coding::pinned::grep(
        &fixture.ctx(),
        json!({ "pattern": "needle", "case": false }),
    )
    .await
    .expect("grep case false");
    let text = output(result);
    assert!(text.contains("*1:needle"), "case-insensitive: {text}");
    assert!(text.contains("*2:NEEDLE"), "case-insensitive: {text}");
    assert!(text.contains("*3:Needle"), "case-insensitive: {text}");
}

#[tokio::test]
async fn grep_per_file_cap_counts_matches_not_rows() {
    let fixture = Fixture::new();
    // 25 matches 10 lines apart: with CONTEXT_BEFORE=1 / CONTEXT_AFTER=3
    // every match contributes ~6 rows (leading context row, match row,
    // trailing context rows, gap marker), so 25 matches produce ~150 rows.
    // The multi-file cap (MULTI_FILE_PER_FILE_MATCHES=20) must admit 20
    // *matches* with their context, not 20 rows (~4 matches).
    let mut content = String::new();
    for n in 1..=250 {
        if n % 10 == 0 {
            content.push_str(&format!("match line {n}\n"));
        } else {
            content.push_str(&format!("filler line {n}\n"));
        }
    }
    fixture.seed("/workspace/dir/capped.txt", &content).await;

    let result = ironclaw_extension_support::coding::pinned::grep(
        &fixture.ctx(),
        json!({ "pattern": "^match", "path": "dir" }),
    )
    .await
    .expect("grep");
    let text = output(result);
    let match_rows: Vec<&str> = text.lines().filter(|line| line.starts_with('*')).collect();
    assert_eq!(
        match_rows.len(),
        20,
        "cap counts matches, not rows:\n{text}"
    );
    assert!(
        match_rows
            .last()
            .is_some_and(|row| row.starts_with("*200:")),
        "twentieth match admitted: {text}"
    );
    // The last admitted match keeps its complete trailing context…
    assert!(
        text.contains(" 203:filler line 203"),
        "trailing context kept: {text}"
    );
    // …the next match and its context are excluded…
    assert!(!text.contains("*210:"), "next match excluded: {text}");
    assert!(
        !text.contains(" 209:filler line 209"),
        "next match's context excluded: {text}"
    );
    // …and the section never ends on a gap marker.
    let last_line = text.lines().last().expect("output");
    assert_ne!(last_line, "...", "no trailing gap marker: {text}");
}

// ─── 7. Differential seam over golden error templates ───────────────────────

#[test]
fn differential_seam_error_templates_agree_with_fixture_rendering() {
    use support::pinned_coding_contract::error_entries;

    let edit_errors = error_entries("edit");
    assert!(!edit_errors.is_empty());
    let mismatches = compare_cases(
        &edit_errors,
        |_, entry| entry.case.clone(),
        |entry| Ok(json!({ "rendered": render_edit_template(&entry.case) })),
        |entry| Ok(json!({ "rendered": expected_edit_render(entry) })),
    );
    assert!(
        mismatches.is_empty(),
        "template render mismatches: {mismatches:#?}"
    );
}

/// Substitute `${name}` placeholders in a golden template.
fn substitute_template(template: &str, values: &[(&str, &str)]) -> String {
    let mut text = template.to_string();
    for (name, value) in values {
        text = text.replace(&format!("${{{name}}}"), value);
    }
    text
}

/// The fixture-derived expected render for `case`: the `example` field when
/// the engine's render is the complete pinned message, the substituted
/// `template` when the pinned renderer keeps instruction tokens literal
/// (`[path#newhash]` / `[path#tag]`), or the example plus the pinned
/// counted-range extension sentence.
fn expected_edit_render(entry: &support::pinned_coding_contract::ErrorEntry) -> String {
    let example = entry
        .example
        .as_deref()
        .expect("fixture example for edit error");
    match entry.case.as_str() {
        "stale_anchor_hash_recognized" => substitute_template(
            &entry.template,
            &[
                ("path", "src/foo.ts"),
                ("expectedFileHash", "1A2B"),
                ("actualFileHash", "3C4D"),
            ],
        ),
        "stale_anchor_hash_unrecognized" => substitute_template(
            &entry.template,
            &[
                ("path", "src/foo.ts"),
                ("expectedFileHash", "9F00"),
                ("actualFileHash", "3C4D"),
            ],
        ),
        "malformed_line_reference" => example.to_string(),
        "line_out_of_bounds" => example.to_string(),
        // The pinned message extends the leading sentence with the
        // counted-range retry form (`messages.ts`: countedEnd = start+end-1
        // → `PUT 10.=24:` for start 10 / end 15).
        "invalid_absolute_range" => {
            format!("{example} For 15 lines starting at 10, use `PUT 10.=24:`.")
        }
        "per_file_failure_aggregate" => example.to_string(),
        "files_not_applied" => example.to_string(),
        "auto_piped_bare_body_rows" => example.to_string(),
        other => panic!("unhandled template case {other}"),
    }
}

/// Render a golden edit error case with representative values through the
/// engine's pinned render functions.
fn render_edit_template(case: &str) -> String {
    match case {
        "stale_anchor_hash_recognized" => {
            // Empty file lines / anchors: the mismatch renderer appends no
            // anchored-context block, leaving exactly the two header lines.
            harness::render_stale_anchor(Some("src/foo.ts"), "1A2B", "3C4D", &[], &[], true)
        }
        "stale_anchor_hash_unrecognized" => {
            harness::render_stale_anchor(Some("src/foo.ts"), "9F00", "3C4D", &[], &[], false)
        }
        "malformed_line_reference" => harness::render_malformed_line_reference("abc"),
        "line_out_of_bounds" => harness::render_line_out_of_bounds(500, 42),
        "invalid_absolute_range" => harness::render_invalid_absolute_range(3, 10, 15),
        "per_file_failure_aggregate" => harness::render_per_file_failure(
            "src/foo.ts",
            "Line 500 does not exist (file has 42 lines)",
        ),
        "files_not_applied" => harness::render_files_not_applied("src/a.ts, src/b.ts"),
        "auto_piped_bare_body_rows" => harness::render_auto_piped_warning(),
        other => panic!("unhandled template case {other}"),
    }
}

// ─── Run-id isolation ───────────────────────────────────────────────────────

#[tokio::test]
async fn read_in_one_run_never_authorizes_edit_in_another() {
    let fixture = Fixture::new();
    fixture.seed("/workspace/foo.ts", "line1\nline2\n").await;
    let tag = fixture.read_tag("foo.ts").await;

    let run_b = fixture.ctx_with_run(Some(RunId::new()));
    let error = ironclaw_extension_support::coding::pinned::edit(
        &run_b,
        json!({ "input": format!("[foo.ts#{tag}]\nPUT 2:\n+new\n") }),
    )
    .await
    .expect_err("cross-run anchor rejected");
    assert_eq!(
        error.kind(),
        CodingEngineErrorKind::StaleAnchorHashUnrecognized
    );
    assert!(
        error_message(&error).contains(&format!("hash #{tag} is not from this session.")),
        "{}",
        error_message(&error)
    );
}

// ─── Pinned bash engine ────────────────────────────────────────────────────

/// Scripted placement-neutral command executor for the pinned bash engine.
struct ScriptedExecutor {
    script: std::collections::HashMap<String, Result<(String, i64), String>>,
}

#[async_trait]
impl ironclaw_host_api::process::CommandExecutor for ScriptedExecutor {
    async fn run_command(
        &self,
        request: ironclaw_host_api::process::CommandExecutionRequest,
    ) -> Result<
        ironclaw_host_api::process::CommandExecutionOutput,
        ironclaw_host_api::process::RuntimeProcessError,
    > {
        let outcome = self.script.get(&request.command).cloned().ok_or_else(|| {
            ironclaw_host_api::process::RuntimeProcessError::ExecutionFailed(format!(
                "unscripted command: {}",
                request.command
            ))
        })?;
        match outcome {
            Ok((output, exit_code)) => Ok(ironclaw_host_api::process::CommandExecutionOutput {
                output,
                saved_output: None,
                exit_code,
                sandboxed: false,
                duration: std::time::Duration::from_millis(1500),
            }),
            Err(reason) => {
                Err(ironclaw_host_api::process::RuntimeProcessError::ExecutionFailed(reason))
            }
        }
    }
}

fn bash_ctx(executor: ScriptedExecutor) -> CodingEngineContext {
    let mut ctx = Fixture::new().ctx();
    ctx.process = Some(Arc::new(executor));
    ctx
}

#[tokio::test]
async fn bash_runs_command_and_renders_omp_notices() {
    let mut script = std::collections::HashMap::new();
    script.insert("echo hello".to_string(), Ok(("hello".to_string(), 0)));
    let result = ironclaw_extension_support::coding::pinned::bash(
        &bash_ctx(ScriptedExecutor { script }),
        json!({ "command": "echo hello" }),
    )
    .await
    .expect("bash runs");

    assert_eq!(output(result.clone()), "hello\n\nWall time: 1.50 seconds");
}

#[tokio::test]
async fn bash_renders_failed_exit_notice() {
    let mut script = std::collections::HashMap::new();
    script.insert("false".to_string(), Ok(("".to_string(), 1)));
    let result = ironclaw_extension_support::coding::pinned::bash(
        &bash_ctx(ScriptedExecutor { script }),
        json!({ "command": "false" }),
    )
    .await
    .expect("bash runs");

    assert_eq!(
        output(result.clone()),
        "(no output)\n\nWall time: 1.50 seconds\n\nCommand exited with code 1"
    );
}

#[tokio::test]
async fn bash_denies_critical_patterns_before_execution() {
    let mut script = std::collections::HashMap::new();
    script.insert("rm -rf /".to_string(), Ok(("".to_string(), 0)));
    let error = ironclaw_extension_support::coding::pinned::bash(
        &bash_ctx(ScriptedExecutor { script }),
        json!({ "command": "rm -rf /" }),
    )
    .await
    .expect_err("critical pattern denied");

    assert_eq!(error.kind(), CodingEngineErrorKind::Input);
    assert!(
        error_message(&error).contains("Blocked by bash pattern"),
        "{}",
        error_message(&error)
    );
}

#[tokio::test]
async fn bash_passes_env_timeout_and_cwd() {
    let mut script = std::collections::HashMap::new();
    script.insert("env".to_string(), Ok(("FOO=bar".to_string(), 0)));
    let result = ironclaw_extension_support::coding::pinned::bash(
        &bash_ctx(ScriptedExecutor { script }),
        json!({
            "command": "env",
            "env": { "FOO": "bar" },
            "timeout": 30,
            "cwd": "/workspace"
        }),
    )
    .await
    .expect("bash runs");

    assert!(output(result.clone()).contains("FOO=bar"));
}

#[tokio::test]
async fn bash_rejects_invalid_env_names() {
    let error = ironclaw_extension_support::coding::pinned::bash(
        &bash_ctx(ScriptedExecutor {
            script: std::collections::HashMap::new(),
        }),
        json!({ "command": "echo hi", "env": { "BAD-NAME": "x" } }),
    )
    .await
    .expect_err("invalid env name rejected");

    assert!(
        error_message(&error).contains("Invalid bash env name"),
        "{}",
        error_message(&error)
    );
}

#[tokio::test]
async fn bash_requires_command() {
    let error = ironclaw_extension_support::coding::pinned::bash(
        &bash_ctx(ScriptedExecutor {
            script: std::collections::HashMap::new(),
        }),
        json!({}),
    )
    .await
    .expect_err("missing command rejected");

    assert!(
        error_message(&error).contains("requires a string `command`"),
        "{}",
        error_message(&error)
    );
}
