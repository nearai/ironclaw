//! Terminal-rendering helpers shared by this crate's two CLI surfaces.
//!
//! Both `extension_lifecycle_command` and `ironhub::render` print
//! extension-supplied text — names, descriptions, versions, publisher strings —
//! to a terminal. That text is untrusted: it arrives from a package manifest or
//! from the hub's HTTP responses, so it can carry ANSI escape sequences, cursor
//! movement, or line-rewriting control characters.
//!
//! [`terminal_safe`] is therefore a **security** helper, not a formatting one,
//! and that is why it lives here instead of being written once per call site.
//! It was previously duplicated verbatim in both modules, which meant a future
//! hardening fix — a tighter escape, a length bound — applied to one copy would
//! silently leave the other rendering unescaped. Raised in review on #7000.
//!
//! Any change to the escaping rule belongs here, and `terminal_safe_escapes_*`
//! below is the pin that keeps the two surfaces agreeing.

/// Render untrusted text safely for a terminal by escaping every character
/// that is not printable ASCII.
///
/// `char::escape_default` renders control characters (including `\x1b`, the
/// ANSI escape introducer), quotes, and backslashes in their `\x..` / `\u{..}`
/// source forms, so no byte in the output can move the cursor, change colour,
/// or rewrite a previous line.
pub(crate) fn terminal_safe(value: &str) -> String {
    value.chars().flat_map(char::escape_default).collect()
}

/// Append one formatted line to `output`.
pub(crate) fn push_line(output: &mut String, args: std::fmt::Arguments<'_>) {
    use std::fmt::Write as _;
    #[allow(clippy::let_underscore_must_use)] // writing to a String is infallible
    let _ = output.write_fmt(args);
    output.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The escaping is what stops extension-supplied text from driving the
    /// operator's terminal, so the dangerous shapes are pinned literally rather
    /// than asserted as "does not contain an escape".
    #[test]
    fn terminal_safe_escapes_control_sequences_from_untrusted_text() {
        // ANSI colour/cursor sequences must lose their introducer.
        let coloured = terminal_safe("\x1b[31mred\x1b[0m");
        assert!(
            !coloured.contains('\x1b'),
            "the ESC introducer must not survive: {coloured:?}"
        );
        assert_eq!(coloured, "\\u{1b}[31mred\\u{1b}[0m");

        // Carriage return can rewrite a line the operator already read.
        assert_eq!(terminal_safe("safe\rEVIL"), "safe\\rEVIL");
        // Newline can forge additional output lines.
        assert_eq!(terminal_safe("a\nb"), "a\\nb");
        // A bare backspace can erase preceding context.
        assert_eq!(terminal_safe("ab\x08c"), "ab\\u{8}c");

        // Ordinary text is untouched, so the escaping stays invisible in the
        // overwhelmingly common case.
        assert_eq!(terminal_safe("slack 1.2.3"), "slack 1.2.3");
    }

    #[test]
    fn push_line_appends_exactly_one_newline() {
        let mut out = String::new();
        push_line(&mut out, format_args!("{}-{}", "a", 1));
        push_line(&mut out, format_args!("b"));
        assert_eq!(out, "a-1\nb\n");
    }
}
