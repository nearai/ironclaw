//! Shared test-only rendering helper for `ui::*` widget tests. Mirrors
//! `app/test_support.rs`'s shape: one place for boilerplate so each
//! submodule's test list stays focused on behavior.

use ratatui::buffer::Buffer;

/// Joins every cell's symbol row-major into one string, so a test can assert
/// on substrings without pinning exact whitespace/column layout (per the
/// plan: "buffer_text ... NO whitespace-pinned adjacency asserts").
pub(crate) fn buffer_text(buf: &Buffer) -> String {
    let area = buf.area();
    let mut out = String::new();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}
