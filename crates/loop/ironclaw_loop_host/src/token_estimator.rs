#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct EstimatedTokenCount(u64);

impl EstimatedTokenCount {
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

pub const CHARS_PER_TOKEN_DEFAULT: u64 = 4;
const BYTES_PER_TOKEN_SAFETY_BOUND: u64 = 2;

pub fn estimate_tokens_from_chars(content: &str) -> EstimatedTokenCount {
    if content.is_empty() {
        return EstimatedTokenCount(0);
    }
    let chars = content.chars().count() as u64;
    let char_estimate = chars.div_ceil(CHARS_PER_TOKEN_DEFAULT);
    let byte_estimate = (content.len() as u64).div_ceil(BYTES_PER_TOKEN_SAFETY_BOUND);
    EstimatedTokenCount(char_estimate.max(byte_estimate).max(1))
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
    fn estimate_uses_byte_bound_for_unicode_dense_input() {
        assert_eq!(estimate_tokens_from_chars("你好世界").as_u64(), 6);
    }

    #[test]
    fn estimate_uses_ceiling_division() {
        assert_eq!(estimate_tokens_from_chars("abcde").as_u64(), 3);
    }
}
