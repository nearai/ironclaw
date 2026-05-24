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
