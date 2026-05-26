//! Embedded asset bytes.
//!
//! Populated at compile time by `build.rs` from
//! `crates/ironclaw_webui_v2_static/static/`. Each file becomes one
//! `Asset` row keyed by its URL path (relative to the `/v2` mount
//! prefix). `index.html` is handled separately — see
//! [`INDEX_HTML_TEMPLATE`].

pub(crate) struct Asset {
    pub bytes: &'static [u8],
    pub content_type: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/assets_generated.rs"));

pub(crate) fn lookup(path: &str) -> Option<&'static Asset> {
    // Path table is sorted at build time; binary search keeps the
    // per-request work O(log n) without pulling in a hash map.
    ASSETS
        .binary_search_by(|(p, _)| (*p).cmp(path))
        .ok()
        .map(|idx| &ASSETS[idx].1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_returns_none_for_unknown_path() {
        // Direct coverage of the `None` arm. The router-level tests
        // exercise the `Some` path via known assets and the SPA-shell
        // fallback for unknown paths, but neither directly asserts
        // that the asset table itself returns `None` — a future
        // refactor that swaps `binary_search_by` for something that
        // returns the closest match instead would regress this
        // contract silently without this guard.
        assert!(lookup("nonexistent.js").is_none());
        assert!(lookup("../etc/passwd").is_none());
        assert!(lookup("").is_none());
    }
}
