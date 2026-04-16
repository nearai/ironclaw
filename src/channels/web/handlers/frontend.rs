//! Frontend extension API handlers.
//!
//! Provides endpoints for reading/writing layout configuration and
//! discovering/serving widget files from the workspace. All gateway state
//! lives under `.system/gateway/` in the workspace, alongside other
//! `.system/*` subsystems.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::{StatusCode, header},
    response::IntoResponse,
};

use ironclaw_gateway::{LayoutConfig, ResolvedWidget, WidgetManifest, is_safe_widget_id};

use crate::channels::web::auth::{AdminUser, AuthenticatedUser};
use crate::channels::web::handlers::memory::resolve_workspace;
use crate::channels::web::server::GatewayState;
use crate::workspace::Workspace;

/// Workspace path to the layout config document.
const LAYOUT_PATH: &str = ".system/gateway/layout.json";

/// Workspace directory containing widget subdirectories. Trailing slash is
/// kept so it can be passed straight to `Workspace::list()`.
const WIDGETS_DIR: &str = ".system/gateway/widgets/";

/// Read and parse `.system/gateway/layout.json` from the workspace.
///
/// * Missing file → returns [`LayoutConfig::default`] silently. A workspace
///   with no customizations is the common case and shouldn't generate log
///   noise.
/// * Malformed JSON → logs a `warn!` with the parse error and falls back to
///   the default. A broken file must never be allowed to crash a page load.
///
/// Single source of truth for layout reads: both
/// [`frontend_layout_handler`] (the public `GET /api/frontend/layout`
/// endpoint) and `build_frontend_html` in
/// `src/channels/web/server.rs` call through here so a future change to the
/// fallback / parse / warning behavior only needs to land in one place.
pub async fn read_layout_config(workspace: &Workspace) -> LayoutConfig {
    match workspace.read(LAYOUT_PATH).await {
        Ok(doc) => match serde_json::from_str(&doc.content) {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = LAYOUT_PATH,
                    "layout.json is invalid — falling back to default layout"
                );
                LayoutConfig::default()
            }
        },
        // A workspace with no `.system/gateway/layout.json` is the common
        // case (no customizations) and must stay silent — every page load
        // hits this path. Any OTHER error variant (IoError, SearchFailed,
        // backend connectivity, etc.) is unexpected and would otherwise
        // silently drop customizations without any operator signal; log
        // it at warn! so backend problems surface even though the caller
        // falls back to the default layout either way.
        Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => LayoutConfig::default(),
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = LAYOUT_PATH,
                "workspace read failed — falling back to default layout \
                 (customizations may be silently skipped)"
            );
            LayoutConfig::default()
        }
    }
}

/// `GET /api/frontend/layout` — return the current layout configuration.
///
/// Thin wrapper over [`read_layout_config`].
pub async fn frontend_layout_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<LayoutConfig>, (StatusCode, String)> {
    let workspace = resolve_workspace(&state, &user, None).await?;
    Ok(Json(read_layout_config(&workspace).await))
}

/// `PUT /api/frontend/layout` — update the layout configuration.
///
/// Writes the provided layout config to `.system/gateway/layout.json`.
///
/// **Admin-only.** Layout changes are global in single-tenant mode and
/// shape what every user of the gateway sees: branding, hidden tabs,
/// disabled widgets. Allowing any `member`-role token to call this
/// endpoint would let a low-privilege account effectively deface the UI
/// for the operator. Locked down to `AdminUser` so the same role gate
/// that protects user management and secrets management also protects
/// the chrome of the page itself. In multi-tenant mode this still
/// resolves the per-user workspace via `resolve_workspace`, so admins
/// configuring their own tenant get the expected behavior.
pub async fn frontend_layout_update_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(user): AdminUser,
    Json(layout): Json<LayoutConfig>,
) -> Result<StatusCode, (StatusCode, String)> {
    let workspace = resolve_workspace(&state, &user, None).await?;

    let content = serde_json::to_string_pretty(&layout).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid layout config: {e}"),
        )
    })?;

    workspace.write(LAYOUT_PATH, &content).await.map_err(|e| {
        tracing::error!("Failed to write layout config: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to write layout config".to_string(),
        )
    })?;

    Ok(StatusCode::OK)
}

/// `GET /api/frontend/widgets` — list all widget manifests.
///
/// Scans `.system/gateway/widgets/` in the workspace for directories
/// containing `manifest.json` and returns their parsed manifests.
pub async fn frontend_widgets_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<Vec<WidgetManifest>>, (StatusCode, String)> {
    let workspace = resolve_workspace(&state, &user, None).await?;
    let manifests = load_widget_manifests(&workspace).await;
    Ok(Json(manifests))
}

/// Discover every widget in `.system/gateway/widgets/` and return its parsed
/// manifest. Malformed manifests are skipped with a `warn!` log.
pub(crate) async fn load_widget_manifests(workspace: &Workspace) -> Vec<WidgetManifest> {
    // A missing / empty widgets directory is the common case and the
    // workspace returns an empty `Vec` for it. An actual `Err` here means
    // the backend listing call failed (IoError, connectivity, etc.); the
    // caller (`/api/frontend/widgets`) would otherwise return `200 []`
    // and hide the real problem. Log at warn! before the empty-list
    // fallback so operators notice.
    let entries = match workspace.list(WIDGETS_DIR).await {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = WIDGETS_DIR,
                "workspace list failed — returning empty widget list \
                 (installed widgets may be silently skipped)"
            );
            Vec::new()
        }
    };

    let mut manifests = Vec::new();
    for entry in entries {
        if !entry.is_directory {
            continue;
        }
        if let Some(manifest) = read_widget_manifest(workspace, entry.name()).await {
            manifests.push(manifest);
        }
    }
    manifests
}

/// Read and parse a single widget's `manifest.json`. Returns `None` (with a
/// `warn!`) for parse failures and `None` silently when the file is missing.
///
/// Validates the on-disk `directory_name` against [`is_safe_widget_id`]
/// BEFORE touching the workspace. The discovery, serving, and runtime
/// contracts all key off the same identifier — `manifest.id` must equal
/// `directory_name` (enforced below), and `manifest.id` must itself pass
/// `is_safe_widget_id` (also enforced below) — so accepting a wider charset
/// at the discovery step than the loader/runtime contract allows would only
/// surface widgets that can never resolve. Using the same validator
/// everywhere keeps discovery, serving (`frontend_widget_file_handler`), and
/// the layout-config gating in lock-step. It also forecloses path-shape
/// payloads (`.`/`..`/backslash/NUL/quotes/whitespace/leading-dash) before
/// they ever get composed into `{WIDGETS_DIR}{directory_name}/...`
/// workspace reads — important for any filesystem-backed `Workspace`
/// implementation that doesn't normalize separator/traversal components.
///
/// Also enforces that `manifest.id` matches the on-disk directory name. The
/// rest of the loader uses `directory_name` to compute file paths
/// (`{WIDGETS_DIR}{directory_name}/index.js` etc.) while layout-config gating
/// and the public `/api/frontend/widget/{id}/{*file}` endpoint key off
/// `manifest.id`. If those drift, code can be loaded from one folder while
/// the rest of the system thinks the widget lives somewhere else — both a
/// correctness footgun for widget authors and an attack surface for path
/// confusion. Reject the mismatch loudly instead of silently picking one.
async fn read_widget_manifest(
    workspace: &Workspace,
    directory_name: &str,
) -> Option<WidgetManifest> {
    if !is_safe_widget_id(directory_name) {
        tracing::warn!(
            directory = directory_name,
            "skipping widget: directory name is not a safe widget identifier \
             (alphanumeric + `._-`, first char alphanumeric, ≤64 chars)"
        );
        return None;
    }
    let manifest_path = format!("{WIDGETS_DIR}{directory_name}/manifest.json");
    let doc = workspace.read(&manifest_path).await.ok()?;
    let manifest = match serde_json::from_str::<WidgetManifest>(&doc.content) {
        Ok(manifest) => manifest,
        Err(e) => {
            tracing::warn!(
                path = %manifest_path,
                error = %e,
                "skipping widget with invalid manifest"
            );
            return None;
        }
    };
    // Belt-and-braces: even though `manifest.id` is also checked against
    // `directory_name` below, the id flows directly into HTML attributes
    // (`data-widget="<id>"`) and CSS attribute selectors
    // (`scope_css`'s `[data-widget="<id>"]` prefix). The latter has no
    // escape pass — a manifest id like `x"],.evil{color:red}[x` would
    // close the attribute selector and inject arbitrary CSS rules.
    // Validate the id against the same charset rules the directory name
    // already passes (`is_safe_widget_id` is the canonical check) so a
    // hostile id is rejected at load time, before any rendering layer
    // sees it. The reject-then-mismatch-check ordering matters: a hostile
    // id is logged as "unsafe charset" rather than as a directory
    // mismatch, which is the more useful diagnostic.
    if !is_safe_widget_id(&manifest.id) {
        tracing::warn!(
            path = %manifest_path,
            manifest_id = %manifest.id,
            "skipping widget: manifest.id contains characters outside the \
             safe widget identifier charset (alphanumeric + `._-`, ≤64 chars)"
        );
        return None;
    }
    if manifest.id != directory_name {
        tracing::warn!(
            path = %manifest_path,
            directory = directory_name,
            manifest_id = %manifest.id,
            "skipping widget: manifest.id does not match the on-disk directory name"
        );
        return None;
    }
    Some(manifest)
}

/// Discover every widget in `.system/gateway/widgets/` and return the
/// fully-resolved set (manifest + `index.js` + optional `style.css`), filtered
/// by the `enabled` flag in the supplied layout. Widgets missing `index.js`
/// are skipped silently — they're assumed to be in-progress scaffolds.
///
/// This is the single source of truth for widget loading; both the gateway's
/// `/` handler and the `/api/frontend/widgets` handler delegate to it (the
/// latter via [`load_widget_manifests`]).
/// Per-widget size caps. Widget JS/CSS is inlined into every page response
/// (and cached), so a single oversized file bloats every page load. The
/// caps are generous enough for real-world widget bundles but stop a
/// multi-MB file from ending up in the cached HTML.
const MAX_WIDGET_JS_BYTES: usize = 512 * 1024; // 512 KB
const MAX_WIDGET_CSS_BYTES: usize = 256 * 1024; // 256 KB

pub(crate) async fn load_resolved_widgets(
    workspace: &Workspace,
    layout: &LayoutConfig,
) -> Vec<ResolvedWidget> {
    // Same rationale as `load_widget_manifests` above: an empty directory
    // is a normal empty `Vec`, a real `Err` is a backend failure that we
    // shouldn't hide behind an empty widget list on the index page.
    let entries = match workspace.list(WIDGETS_DIR).await {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = WIDGETS_DIR,
                "workspace list failed — rendering index with no widgets \
                 (installed widgets may be silently skipped)"
            );
            Vec::new()
        }
    };

    let mut widgets = Vec::new();
    for entry in entries {
        if !entry.is_directory {
            continue;
        }
        let name = entry.name();
        let Some(manifest) = read_widget_manifest(workspace, name).await else {
            continue;
        };

        // Widgets without `index.js` are incomplete — skip quietly.
        let js_path = format!("{WIDGETS_DIR}{name}/index.js");
        let js = match workspace.read(&js_path).await {
            Ok(doc) => doc.content,
            Err(_) => continue,
        };
        if js.len() > MAX_WIDGET_JS_BYTES {
            tracing::warn!(
                widget = name,
                bytes = js.len(),
                cap = MAX_WIDGET_JS_BYTES,
                "skipping widget: index.js exceeds size cap"
            );
            continue;
        }

        let css = workspace
            .read(&format!("{WIDGETS_DIR}{name}/style.css"))
            .await
            .ok()
            .map(|doc| doc.content)
            .filter(|c| !c.trim().is_empty())
            .filter(|c| {
                if c.len() > MAX_WIDGET_CSS_BYTES {
                    tracing::warn!(
                        widget = name,
                        bytes = c.len(),
                        cap = MAX_WIDGET_CSS_BYTES,
                        "dropping oversized widget style.css"
                    );
                    return false;
                }
                true
            });

        // Respect the layout's `enabled` flag; default is `true` when the
        // widget has no entry at all (see WidgetInstanceConfig::default).
        let enabled = layout
            .widgets
            .get(&manifest.id)
            .map(|w| w.enabled)
            .unwrap_or(true);
        if !enabled {
            continue;
        }

        widgets.push(ResolvedWidget { manifest, js, css });
    }
    widgets
}

/// `GET /api/frontend/widget/{id}/{*file}` — serve a widget file.
///
/// Serves JS/CSS files from `.system/gateway/widgets/{id}/{file}` in the
/// workspace with appropriate MIME types.
pub async fn frontend_widget_file_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path((id, file)): Path<(String, String)>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // The widget id must match the loader/runtime contract enforced by
    // `read_widget_manifest` (`is_safe_widget_id`: alphanumeric + `._-`,
    // first char alphanumeric, ≤64 chars). A looser segment-only check
    // would permit quotes, brackets, whitespace, newlines, etc. — none of
    // which can ever resolve to a real widget (the loader would have
    // rejected the manifest), but they would still produce surprising
    // `.system/gateway/widgets/<weird>/...` workspace paths and inject
    // arbitrary content into the `workspace_path` field of the warn! log
    // below. Lock the accepted charset to the same one the loader uses.
    if !is_safe_widget_id(&id) {
        return Err((StatusCode::BAD_REQUEST, "Invalid widget id".to_string()));
    }
    // The file parameter is a nested path (`*file` wildcard). Validate every
    // `/`-separated component against the same strict charset so neither
    // `a/../b` nor `a/./b` nor `a/\..\b` nor whitespace/quote/control-char
    // payloads slip through. Each component must look like a normal
    // filename (`index.js`, `assets`, `icon.svg`, …).
    if file.is_empty() || file.starts_with('/') || file.contains('\0') {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid widget file path".to_string(),
        ));
    }
    if !file.split('/').all(is_safe_widget_id) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid widget file path".to_string(),
        ));
    }

    let workspace = resolve_workspace(&state, &user, None).await?;
    let path = format!("{WIDGETS_DIR}{id}/{file}");

    // Don't echo the resolved workspace path back to the caller — that
    // leaks the `.system/gateway/widgets/...` layout to anyone probing
    // the endpoint and gives an attacker a free oracle for "what
    // directories exist". Log the full path internally so debugging
    // still works, then return a generic message to the client.
    //
    // Distinguish 404 from 500: a genuine missing file
    // (`DocumentNotFound`) deserves 404, but backend failures (IoError,
    // SearchFailed, connectivity) used to also come out as 404, which
    // turned every workspace outage into a silent stream of "not found"
    // errors that masked the real issue. Map the not-found variant to
    // 404 and route everything else to 500 so operational problems
    // surface in status codes as well as logs. The client-facing body
    // stays generic in both cases to preserve the path-enumeration
    // hardening above.
    let doc = workspace.read(&path).await.map_err(|e| {
        use crate::error::WorkspaceError;
        match e {
            WorkspaceError::DocumentNotFound { .. } => {
                tracing::warn!(
                    workspace_path = %path,
                    "widget file not found"
                );
                (StatusCode::NOT_FOUND, "Widget file not found".to_string())
            }
            other => {
                tracing::warn!(
                    workspace_path = %path,
                    error = %other,
                    "widget file read failed (backend error)"
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to read widget file".to_string(),
                )
            }
        }
    })?;

    // Determine MIME type from the file extension (case-insensitive — the
    // browser doesn't care about `.JS` vs `.js`). Widgets legitimately
    // ship assets beyond JS/CSS (icons, webfonts, source maps); falling
    // back to `text/plain` broke SVG rendering and triggered
    // content-sniffing for the font files. Cover the common widget asset
    // types explicitly and keep `text/plain` as the last-resort fallback.
    let ext = file
        .rsplit('.')
        .next()
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    let content_type = match ext.as_str() {
        // Text formats — served correctly via `doc.content: String`.
        "js" | "mjs" => "application/javascript",
        "css" => "text/css",
        "json" => "application/json",
        "map" => "application/json",
        "svg" => "image/svg+xml",
        // Binary formats — MIME types are mapped so the browser doesn't
        // content-sniff, but `Workspace::read()` returns `String` (UTF-8
        // text), so binary payloads will be silently corrupted until a
        // `read_bytes()` workspace path exists. Widget authors should host
        // binary assets externally or Base64-encode them into CSS/JS.
        // TODO: support binary widget assets via a `read_bytes()` path.
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        _ => "text/plain",
    };

    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        doc.content,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The serving endpoint (`frontend_widget_file_handler`) validates the
    /// `id` and each `file` component with `is_safe_widget_id`. This pins
    /// the contract: well-formed widget asset paths are accepted, hostile
    /// payloads (traversal, separators, quotes, brackets, whitespace,
    /// control chars) are rejected. The check matches what the loader
    /// (`read_widget_manifest`) enforces on `manifest.id`, so the serving
    /// endpoint's accepted charset can never drift wider than the
    /// loader/runtime contract. See PR #1725 review thread r3053351457.
    #[test]
    fn widget_file_path_components_use_strict_charset() {
        // Accepted: normal asset paths a widget would actually ship.
        for ok in [
            "index.js",
            "style.css",
            "assets/icon.svg",
            "i18n/en/strings.json",
        ] {
            let parts: Vec<&str> = ok.split('/').collect();
            assert!(
                parts.iter().all(|p| is_safe_widget_id(p)),
                "expected {ok:?} to pass per-component is_safe_widget_id"
            );
        }
        // Rejected: traversal and shape-of-path payloads.
        for bad in [
            "../etc/passwd",
            "assets/../secrets",
            "./index.js",
            "assets\\..\\secrets",
            "-flag.js", // first char must be alphanumeric
            ".hidden",  // first char must be alphanumeric
            "name with space",
            "name\nnewline",
            "name\"quote",
            "name[bracket",
            "name\0nul",
        ] {
            let parts: Vec<&str> = bad.split('/').collect();
            assert!(
                !parts.iter().all(|p| is_safe_widget_id(p)),
                "expected {bad:?} to fail per-component is_safe_widget_id"
            );
        }
    }

    #[cfg(feature = "libsql")]
    mod widget_loader {
        use super::*;
        use crate::db::libsql::LibSqlBackend;
        use std::sync::Arc;

        async fn make_workspace() -> (Workspace, tempfile::TempDir) {
            let dir = tempfile::tempdir().expect("tempdir");
            let backend = LibSqlBackend::new_local(&dir.path().join("widget_loader.db"))
                .await
                .expect("libsql backend");
            <LibSqlBackend as crate::db::Database>::run_migrations(&backend)
                .await
                .expect("migrations");
            let db: Arc<dyn crate::db::Database> = Arc::new(backend);
            (Workspace::new_with_db("widget_loader", db), dir)
        }

        async fn write_widget(ws: &Workspace, dir: &str, manifest_id: &str) {
            let manifest = serde_json::json!({
                "id": manifest_id,
                "name": "Test",
                "slot": "tab",
            });
            ws.write(
                &format!("{WIDGETS_DIR}{dir}/manifest.json"),
                &manifest.to_string(),
            )
            .await
            .expect("write manifest");
            ws.write(&format!("{WIDGETS_DIR}{dir}/index.js"), "/* test */")
                .await
                .expect("write index.js");
        }

        /// Regression: a widget whose `manifest.id` does not match the
        /// directory name must be skipped. Otherwise the loader can mount
        /// code from one folder under a different id, and
        /// `/api/frontend/widget/{id}/{*file}` (which keys off the id) will
        /// silently 404 because it looks under the wrong directory.
        #[tokio::test]
        async fn skips_widget_when_manifest_id_does_not_match_directory() {
            let (ws, _dir) = make_workspace().await;
            write_widget(&ws, "real-id", "spoofed-id").await;

            let manifest = read_widget_manifest(&ws, "real-id").await;
            assert!(
                manifest.is_none(),
                "widget with mismatched id must be rejected"
            );

            let layout = LayoutConfig::default();
            let resolved = load_resolved_widgets(&ws, &layout).await;
            assert!(
                resolved.is_empty(),
                "load_resolved_widgets must skip mismatched widgets"
            );

            let manifests = load_widget_manifests(&ws).await;
            assert!(
                manifests.is_empty(),
                "load_widget_manifests must skip mismatched widgets"
            );
        }

        /// Regression: a directory name that fails `is_safe_widget_id`
        /// must be skipped before any path is composed. Covers the classic
        /// path-shape payloads (`.`, `..`, embedded `/`, embedded `\`,
        /// NUL) and the wider charset that the previous `is_safe_segment`
        /// check used to permit but the loader/runtime contract has
        /// always rejected: leading-dash, leading-dot, quotes, brackets,
        /// whitespace, control chars. Pinning the discovery validator to
        /// `is_safe_widget_id` keeps it in lock-step with
        /// `frontend_widget_file_handler` and `manifest.id` validation,
        /// so a filesystem-backed `Workspace` implementation that didn't
        /// normalize entry names couldn't be tricked into reading
        /// outside the widgets subtree, and the discovery layer never
        /// surfaces a directory whose name can never become a valid id.
        #[tokio::test]
        async fn skips_widget_with_unsafe_directory_name() {
            let (ws, _dir) = make_workspace().await;

            // `read_widget_manifest` is the chokepoint both call sites
            // share, so directly probing it covers both
            // `load_widget_manifests` and `load_resolved_widgets`.
            //
            // First group: classic path-shape payloads — the previous
            // `is_safe_segment` validator already rejected these.
            // Second group: shapes the previous validator wrongly
            // permitted (`-flag`, `.hidden`, `name with space`, etc.) —
            // these can never resolve as widget ids per
            // `is_safe_widget_id` and must now also be rejected at the
            // discovery step rather than caught later by the
            // `manifest.id` charset / mismatch check.
            for unsafe_name in [
                // path-shape payloads
                "..",
                ".",
                "a/b",
                "a\\b",
                "evil\0name",
                // wider charset that fails is_safe_widget_id
                "-flag",
                ".hidden",
                "name with space",
                "name\"quote",
                "name[bracket",
                "name\nnewline",
            ] {
                let manifest = read_widget_manifest(&ws, unsafe_name).await;
                assert!(
                    manifest.is_none(),
                    "directory name {unsafe_name:?} must be rejected by \
                     is_safe_widget_id"
                );
            }
        }

        /// Regression for the paranoid review's P-W4 / P-H10 finding:
        /// a manifest whose `id` would inject CSS or HTML must be
        /// rejected at load time, even if the on-disk directory name
        /// passes `is_safe_widget_id`. The id flows directly into
        /// `[data-widget="<id>"]` in `scope_css` (no escape pass) and
        /// into `data-widget="<id>"` HTML attributes — the
        /// type-level check `is_safe_widget_id` makes both vectors
        /// impossible regardless of the rendering layer.
        #[tokio::test]
        async fn skips_widget_when_manifest_id_fails_charset_check() {
            let (ws, _dir) = make_workspace().await;
            // Directory name is a perfectly valid segment...
            let dir_name = "evil";
            // ...but the manifest id is the CSS-selector breakout
            // payload from serrrfirat's P-W4 example.
            let manifest = serde_json::json!({
                "id": "x\"],.evil{color:red}[x",
                "name": "Evil",
                "slot": "tab",
            });
            ws.write(
                &format!("{WIDGETS_DIR}{dir_name}/manifest.json"),
                &manifest.to_string(),
            )
            .await
            .expect("write manifest");
            ws.write(&format!("{WIDGETS_DIR}{dir_name}/index.js"), "/* test */")
                .await
                .expect("write index.js");

            assert!(
                read_widget_manifest(&ws, dir_name).await.is_none(),
                "manifest with CSS-selector-breakout id must be rejected"
            );
            assert!(
                load_resolved_widgets(&ws, &LayoutConfig::default())
                    .await
                    .is_empty(),
                "load_resolved_widgets must skip charset-failing widgets"
            );
        }

        /// Sanity check: matching id + directory mounts normally.
        #[tokio::test]
        async fn loads_widget_when_manifest_id_matches_directory() {
            let (ws, _dir) = make_workspace().await;
            write_widget(&ws, "skills-viewer", "skills-viewer").await;

            let resolved = load_resolved_widgets(&ws, &LayoutConfig::default()).await;
            assert_eq!(resolved.len(), 1);
            assert_eq!(resolved[0].manifest.id, "skills-viewer");
        }
    }
}
