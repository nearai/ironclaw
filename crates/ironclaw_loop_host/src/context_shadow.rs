/// Deterministic shadow token estimator for prompt-surface diagnostics.
#[allow(clippy::manual_div_ceil)]
pub(crate) fn estimate_tokens(s: &str) -> u32 {
    let char_count = s.chars().count();
    ((char_count + 3) / 4).min(u32::MAX as usize) as u32
}

#[cfg(test)]
mod tests {
    use super::estimate_tokens;

    #[test]
    fn estimate_tokens_uses_ceiling_char_count_over_four() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("a"), 1);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
    }

    #[test]
    fn estimate_tokens_counts_chars_not_bytes() {
        assert_eq!(estimate_tokens("ééééé"), 2);
    }
}
