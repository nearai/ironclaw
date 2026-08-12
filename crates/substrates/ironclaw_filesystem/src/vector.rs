//! Shared vector helpers used by every backend that serves
//! `Filter::VectorNearest` via brute-force cosine ranking.
//!
//! Backends store embeddings as little-endian `f32` blobs under an
//! `IndexValue::Bytes` projection. Decoding and similarity scoring is
//! identical across backends; centralising the implementation here keeps the
//! three implementations from drifting (the in-memory, libSQL, and Postgres
//! backends each used to carry their own byte-identical copy).

/// Decode a little-endian `f32` blob written by an `IndexValue::Bytes`
/// projection. Returns `None` if the blob is empty or has a length that
/// isn't a multiple of `f32`'s byte size.
pub(crate) fn decode_embedding_blob(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(std::mem::size_of::<f32>()) {
        return None;
    }
    Some(
        bytes
            .chunks_exact(std::mem::size_of::<f32>())
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect(),
    )
}

/// Cosine similarity between two equal-length vectors. Returns `None` for
/// mismatched lengths, empty inputs, zero-magnitude vectors, or non-finite
/// scores (NaN / inf). Backends treat `None` as "no rank" and drop the
/// candidate from the result set.
pub(crate) fn cosine_similarity(left: &[f32], right: &[f32]) -> Option<f32> {
    if left.len() != right.len() || left.is_empty() {
        return None;
    }
    let mut dot = 0.0_f32;
    let mut left_norm = 0.0_f32;
    let mut right_norm = 0.0_f32;
    for (l, r) in left.iter().zip(right.iter()) {
        dot += l * r;
        left_norm += l * l;
        right_norm += r * r;
    }
    if left_norm <= 0.0 || right_norm <= 0.0 {
        return None;
    }
    let score = dot / (left_norm.sqrt() * right_norm.sqrt());
    if score.is_finite() { Some(score) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_rejects_misaligned_and_empty_blobs() {
        assert_eq!(decode_embedding_blob(&[]), None);
        assert_eq!(decode_embedding_blob(&[0u8, 1, 2]), None);
        let expected = vec![1.0_f32, -1.0];
        let bytes: Vec<u8> = expected.iter().flat_map(|f| f.to_le_bytes()).collect();
        assert_eq!(decode_embedding_blob(&bytes), Some(expected));
    }

    #[test]
    fn cosine_handles_zero_and_nan_inputs() {
        assert_eq!(cosine_similarity(&[], &[]), None);
        assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 1.0]), None);
        assert_eq!(cosine_similarity(&[1.0], &[1.0, 2.0]), None);
        let identical = cosine_similarity(&[1.0, 0.0, 0.0], &[1.0, 0.0, 0.0]).unwrap();
        assert!((identical - 1.0).abs() < 1e-6);
    }
}
