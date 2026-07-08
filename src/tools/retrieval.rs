//! Per-turn semantic tool retrieval: rank tools by similarity to the message.

/// Cosine similarity. Returns 0.0 if the vectors differ in length or are empty/zero.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (ai, bi) in a.iter().zip(b.iter()) {
        dot += ai * bi;
        na += ai * ai;
        nb += bi * bi;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Rank `items` (key, vector) by cosine to `query`; keep score >= `min_score`,
/// highest first, capped at `k`. Returns the keys.
pub fn rank_top_k(
    query: &[f32],
    items: &[(String, Vec<f32>)],
    k: usize,
    min_score: f32,
) -> Vec<String> {
    let mut scored: Vec<(&String, f32)> = items
        .iter()
        .map(|(key, vec)| (key, cosine(query, vec)))
        .filter(|(_, s)| *s >= min_score)
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored
        .into_iter()
        .take(k)
        .map(|(key, _)| key.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cosine_and_ranking() {
        // orthogonal -> 0, identical -> 1
        assert!((cosine(&[1.0, 0.0], &[0.0, 1.0])).abs() < 1e-6);
        assert!((cosine(&[1.0, 1.0], &[1.0, 1.0]) - 1.0).abs() < 1e-6);
        let items = vec![
            ("trip".to_string(), vec![1.0, 0.0]),
            ("ocr".to_string(), vec![0.0, 1.0]),
            ("place".to_string(), vec![0.9, 0.1]),
        ];
        // query aligned with the "trip"/"place" axis; k=2, floor 0.5
        let got = rank_top_k(&[1.0, 0.0], &items, 2, 0.5);
        assert_eq!(got, vec!["trip".to_string(), "place".to_string()]);
    }
    #[test]
    fn min_score_floor_excludes_weak_and_k_caps() {
        let items = vec![
            ("a".to_string(), vec![1.0, 0.0]),
            ("b".to_string(), vec![0.2, 0.98]),
        ];
        assert_eq!(
            rank_top_k(&[1.0, 0.0], &items, 5, 0.5),
            vec!["a".to_string()]
        );
    }
}
