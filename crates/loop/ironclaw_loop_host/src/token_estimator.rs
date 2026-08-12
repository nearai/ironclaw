#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct EstimatedTokenCount(u64);

impl EstimatedTokenCount {
    pub fn as_u64(self) -> u64 {
        self.0
    }

    pub fn saturating_as_u32(self) -> u32 {
        u32::try_from(self.0).unwrap_or(u32::MAX)
    }
}

pub const CHARS_PER_TOKEN_DEFAULT: u64 = 4;
const TOKEN_UNITS_PER_NON_ASCII_BYTE: u64 = 2;

pub fn estimate_tokens_from_chars(content: &str) -> EstimatedTokenCount {
    if content.is_empty() {
        return EstimatedTokenCount(0);
    }
    let token_units = content.chars().fold(0_u64, |total, character| {
        let character_units = if character.is_ascii() {
            1
        } else {
            (character.len_utf8() as u64).saturating_mul(TOKEN_UNITS_PER_NON_ASCII_BYTE)
        };
        total.saturating_add(character_units)
    });
    EstimatedTokenCount(token_units.div_ceil(CHARS_PER_TOKEN_DEFAULT).max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_returns_zero_for_empty_input() {
        assert_eq!(estimate_tokens_from_chars("").as_u64(), 0);
    }

    #[test]
    fn estimate_returns_one_for_short_non_empty_input() {
        assert_eq!(estimate_tokens_from_chars("a").as_u64(), 1);
    }

    #[test]
    fn estimate_retains_conservative_unicode_surcharge() {
        assert_eq!(estimate_tokens_from_chars("你好世界").as_u64(), 6);
    }

    #[test]
    fn estimate_uses_ceiling_division() {
        assert_eq!(estimate_tokens_from_chars("abcde").as_u64(), 2);
    }

    #[test]
    fn estimate_uses_documented_ascii_rate() {
        let ascii = "a".repeat(4_000);

        assert_eq!(estimate_tokens_from_chars(&ascii).as_u64(), 1_000);
    }

    #[test]
    fn saturating_u32_conversion_caps_large_estimates() {
        assert_eq!(EstimatedTokenCount(u64::MAX).saturating_as_u32(), u32::MAX);
    }
}
