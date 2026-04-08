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

use ironclaw_gateway::{LayoutConfig, ResolvedWidget, WidgetManifest};

use crate::channels::web::auth::AuthenticatedUser;
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
        Err(_) => LayoutConfig::default(),
    }
}

/// `GET /api/frontend/layout` — return the current layout configuration.
///
/// Thin wrapper over [`read_layout_config`].
pub async fn frontend_layout_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<LayoutConfig>, (StatusCode, String)> {
    let workspace = resolve_workspace(&state, &user).await?;
    Ok(Json(read_layout_config(&workspace).await))
}

/// `PUT /api/frontend/layout` — update the layout configuration.
///
/// Writes the provided layout config to `.system/gateway/layout.json`.
pub async fn frontend_layout_update_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(layout): Json<LayoutConfig>,
) -> Result<StatusCode, (StatusCode, String)> {
    let workspace = resolve_workspace(&state, &user).await?;

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
    let workspace = resolve_workspace(&state, &user).await?;
    let manifests = load_widget_manifests(&workspace).await;
    Ok(Json(manifests))
}

/// Discover every widget in `.system/gateway/widgets/` and return its parsed
/// manifest. Malformed manifests are skipped with a `warn!` log.
pub(crate) async fn load_widget_manifests(workspace: &Workspace) -> Vec<WidgetManifest> {
    let entries = workspace.list(WIDGETS_DIR).await.unwrap_or_default();

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
pub(crate) async fn load_resolved_widgets(
    workspace: &Workspace,
    layout: &LayoutConfig,
) -> Vec<ResolvedWidget> {
    let entries = workspace.list(WIDGETS_DIR).await.unwrap_or_default();

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

        let css = workspace
            .read(&format!("{WIDGETS_DIR}{name}/style.css"))
            .await
            .ok()
            .map(|doc| doc.content)
            .filter(|c| !c.trim().is_empty());

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
    // The widget id is a single path segment; it must not contain any
    // separator and must not be `.`, `..`, or empty.
    if !is_safe_segment(&id) {
        return Err((StatusCode::BAD_REQUEST, "Invalid widget id".to_string()));
    }
    // The file parameter is a nested path (`*file` wildcard). Validate every
    // component independently so neither `a/../b` nor `a/./b` nor
    // `a/\..\b` slips through.
    if !is_safe_relative_path(&file) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid widget file path".to_string(),
        ));
    }

    let workspace = resolve_workspace(&state, &user).await?;
    let path = format!("{WIDGETS_DIR}{id}/{file}");

    let doc = workspace.read(&path).await.map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            format!("Widget file not found: {path}"),
        )
    })?;

    // Determine MIME type from the file extension (case-insensitive — the
    // browser doesn't care about `.JS` vs `.js`).
    let ext = file
        .rsplit('.')
        .next()
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    let content_type = match ext.as_str() {
        "js" | "mjs" => "application/javascript",
        "css" => "text/css",
        "json" => "application/json",
        "map" => "application/json",
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

/// True if `s` is a safe single path segment: non-empty, no separators, and
/// not a relative component (`.`/`..`). Also rejects backslash and NUL so
/// platform-specific separators and C-string terminators cannot sneak past.
fn is_safe_segment(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && !s.contains('/')
        && !s.contains('\\')
        && !s.contains('\0')
}

/// True if `s` is a safe relative path under the widget directory — every
/// `/`-separated component must itself pass `is_safe_segment`. Leading or
/// trailing slashes and empty components are rejected.
fn is_safe_relative_path(s: &str) -> bool {
    if s.is_empty() || s.starts_with('/') || s.contains('\0') {
        return false;
    }
    s.split('/').all(is_safe_segment)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_allows_normal_names() {
        assert!(is_safe_segment("widget-1"));
        assert!(is_safe_segment("dashboard_v2"));
        assert!(is_safe_segment("a.b.c"));
        assert!(is_safe_segment("foo..bar")); // `..` embedded in a longer name is fine
    }

    #[test]
    fn segment_rejects_traversal_and_separators() {
        assert!(!is_safe_segment(""));
        assert!(!is_safe_segment("."));
        assert!(!is_safe_segment(".."));
        assert!(!is_safe_segment("a/b"));
        assert!(!is_safe_segment("a\\b"));
        assert!(!is_safe_segment("nul\0byte"));
    }

    #[test]
    fn relative_path_allows_multi_component() {
        assert!(is_safe_relative_path("index.js"));
        assert!(is_safe_relative_path("assets/icon.svg"));
        assert!(is_safe_relative_path("i18n/en/strings.json"));
    }

    #[test]
    fn relative_path_rejects_traversal() {
        assert!(!is_safe_relative_path(""));
        assert!(!is_safe_relative_path("/etc/passwd"));
        assert!(!is_safe_relative_path("assets/../secrets"));
        assert!(!is_safe_relative_path("./index.js"));
        assert!(!is_safe_relative_path("assets//icon.svg"));
        assert!(!is_safe_relative_path("assets\\..\\secrets"));
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
