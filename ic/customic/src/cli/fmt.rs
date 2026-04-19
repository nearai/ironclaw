//! Shared terminal design system.
//!
//! Centralizes color tokens, rendering primitives, and width detection
//! for consistent CLI output. Respects `NO_COLOR` env var and non-TTY
//! output (piping to file, CI, etc.).

use std::io::IsTerminal;

// ── Color detection ─────────────────────────────────────────

/// Returns `true` when ANSI colors should be emitted.
///
/// Disabled when:
/// - `NO_COLOR` env var is set (any value — per <https://no-color.org/>)
/// - stdout is not a terminal (pipe, file redirect, CI)
fn colors_enabled() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    std::io::stdout().is_terminal()
}

/// Returns `true` when the terminal supports 24-bit true-color.
///
/// Checks `$COLORTERM` for `truecolor` or `24bit`.
fn truecolor_enabled() -> bool {
    std::env::var("COLORTERM")
        .map(|v| v.eq_ignore_ascii_case("truecolor") || v.eq_ignore_ascii_case("24bit"))
        .unwrap_or(false)
}

// ── Color tokens ────────────────────────────────────────────

/// Emerald green accent — primary brand color.
///
/// Uses true-color `#34d399` when supported, falls back to basic green.
pub fn accent() -> &'static str {
    if !colors_enabled() {
        return "";
    }
    if truecolor_enabled() {
        "\x1b[38;2;52;211;153m"
    } else {
        "\x1b[32m"
    }
}

/// Bold text.
pub fn bold() -> &'static str {
    if colors_enabled() { "\x1b[1m" } else { "" }
}

/// Green — success indicators.
pub fn success() -> &'static str {
    if colors_enabled() { "\x1b[32m" } else { "" }
}

/// Yellow — warning indicators.
pub fn warning() -> &'static str {
    if colors_enabled() { "\x1b[33m" } else { "" }
}

/// Red — error indicators.
pub fn error() -> &'static str {
    if colors_enabled() { "\x1b[31m" } else { "" }
}

/// Dim gray — labels, secondary text.
pub fn dim() -> &'static str {
    if colors_enabled() { "\x1b[90m" } else { "" }
}

/// Yellow underline — URLs and links.
pub fn link() -> &'static str {
    if colors_enabled() { "\x1b[33;4m" } else { "" }
}

/// Bold accent — commands and interactive elements.
///
/// Uses bold + true-color emerald when supported, falls back to bold green.
pub fn bold_accent() -> &'static str {
    if !colors_enabled() {
        return "";
    }
    if truecolor_enabled() {
        "\x1b[1;38;2;52;211;153m"
    } else {
        "\x1b[1;32m"
    }
}

/// Dim italic — contextual tips and hints.
pub fn hint() -> &'static str {
    if colors_enabled() { "\x1b[2;3m" } else { "" }
}

/// Reset all attributes.
pub fn reset() -> &'static str {
    if colors_enabled() { "\x1b[0m" } else { "" }
}

// ── Width detection ─────────────────────────────────────────

/// Detect terminal width, clamped to [40, 120].
pub fn term_width() -> usize {
    crossterm::terminal::size()
        .map(|(w, _)| w as usize)
        .unwrap_or(80)
        .clamp(40, 120)
}

// ── Rendering primitives ────────────────────────────────────

/// Horizontal separator line (dim `─` characters).
pub fn separator(width: usize) -> String {
    format!("{}{}{}", dim(), "\u{2500}".repeat(width), reset())
}

/// Key-value line with right-padded dim key and accent value.
///
/// ```text
///   Database    libsql (connected)
/// ```
pub fn kv_line(key: &str, value: &str, key_width: usize) -> String {
    format!(
        "  {}{:<width$}{}  {}{}{}",
        dim(),
        key,
        reset(),
        accent(),
        value,
        reset(),
        width = key_width,
    )
}

/// Status icon for check results.
///
/// - `pass` → green `✓`
/// - `fail` → red `✗`
/// - `skip` → dim `○`
pub fn status_icon(kind: StatusKind) -> String {
    match kind {
        StatusKind::Pass => format!("{}\u{2713}{}", success(), reset()),
        StatusKind::Fail => format!("{}\u{2717}{}", error(), reset()),
        StatusKind::Skip => format!("{}\u{25CB}{}", dim(), reset()),
    }
}

/// Kind of status check result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    Pass,
    Fail,
    Skip,
}

/// Top border of a box with an optional label.
///
/// ```text
/// ┌─ label ──────────────────┐
/// ```
pub fn box_top(label: &str, width: usize) -> String {
    if label.is_empty() {
        let fill = width.saturating_sub(2);
        return format!("\u{250C}{}\u{2510}", "\u{2500}".repeat(fill));
    }
    let label_part = format!(" {} ", label);
    // ┌ (1) + ─ (1) + label_part + fill + ┐ (1) = width
    let fill = width.saturating_sub(label_part.len() + 3);
    format!(
        "\u{250C}\u{2500}{}{}{}\u{2510}",
        bold(),
        label_part,
        reset(),
    )
    .replace("\u{2510}", &format!("{}\u{2510}", "\u{2500}".repeat(fill)))
}

/// Content line inside a box.
///
/// ```text
/// │ content                  │
/// ```
pub fn box_line(content: &str, width: usize) -> String {
    let inner = width.saturating_sub(4); // │ + space + space + │
    let padded = if content.len() >= inner {
        content.to_string()
    } else {
        format!("{}{}", content, " ".repeat(inner - content.len()))
    };
    format!("\u{2502} {} \u{2502}", padded)
}

/// Bottom border of a box.
///
/// ```text
/// └──────────────────────────┘
/// ```
pub fn box_bottom(width: usize) -> String {
    let fill = width.saturating_sub(2);
    format!("\u{2514}{}\u{2518}", "\u{2500}".repeat(fill))
}

/// Format a check result line for doctor/status commands.
///
/// ```text
///   ✓ Database          libsql (connected)
///   ✗ Docker            not running — start with: open -a Docker
///   ○ Embeddings        disabled
/// ```
pub fn check_line(kind: StatusKind, name: &str, detail: &str, name_width: usize) -> String {
    format!(
        "  {} {:<width$}  {}",
        status_icon(kind),
        name,
        detail,
        width = name_width,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separator_produces_correct_width() {
        // In test environment NO_COLOR or non-TTY may be active,
        // so strip ANSI to count visible characters.
        let s = separator(10);
        let visible: String = strip_ansi(&s);
        assert_eq!(visible.chars().count(), 10);
    }

    #[test]
    fn kv_line_contains_key_and_value() {
        let line = kv_line("model", "gpt-4o", 12);
        let visible = strip_ansi(&line);
        assert!(visible.contains("model"));
        assert!(visible.contains("gpt-4o"));
    }

    #[test]
    fn status_icon_all_kinds() {
        // Just verify no panic for each variant
        let _ = status_icon(StatusKind::Pass);
        let _ = status_icon(StatusKind::Fail);
        let _ = status_icon(StatusKind::Skip);
    }

    #[test]
    fn box_drawing() {
        let top = box_top("test", 30);
        let line = box_line("content", 30);
        let bottom = box_bottom(30);

        assert!(top.contains('\u{250C}')); // ┌
        assert!(line.contains('\u{2502}')); // │
        assert!(bottom.contains('\u{2514}')); // └
    }

    #[test]
    fn check_line_formatting() {
        let line = check_line(StatusKind::Pass, "Database", "connected", 18);
        let visible = strip_ansi(&line);
        assert!(visible.contains("Database"));
        assert!(visible.contains("connected"));
    }

    #[test]
    fn term_width_in_range() {
        let w = term_width();
        assert!(w >= 40);
        assert!(w <= 120);
    }

    /// Strip ANSI escape sequences for visible-character counting.
    fn strip_ansi(s: &str) -> String {
        let mut result = String::new();
        let mut in_escape = false;
        for c in s.chars() {
            if c == '\x1b' {
                in_escape = true;
                continue;
            }
            if in_escape {
                if c == 'm' {
                    in_escape = false;
                }
                continue;
            }
            result.push(c);
        }
        result
    }
}
