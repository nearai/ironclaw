//! Repeated-output progress policy.
//!
//! Owns the trailing window of completed capability-call OUTPUTS and the
//! dominant-repetition threshold that decides whether output repetition
//! constitutes no progress. See `stop.rs`'s `DefaultStopConditionStrategy`
//! for how this decision composes into the overall stop outcome.

use crate::state::{BoundedRing, CapabilityOutputObservation};

/// Detects loop-detection/diminishing-returns no-progress via dominant
/// (signature, output_digest) repetition in the trailing observation window.
///
/// Requires real OUTPUT repetition, not just a repeated call — see
/// `stop.rs`'s header disclosure for why this is a new mechanism, not a
/// reinstatement of #7531's removed escalation path.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RepeatedOutputProgressStrategy {
    /// Trailing window of `seen_capability_output_digests` observations
    /// scanned for a dominant repeated (signature, output_digest) pair.
    window: usize,
    /// Occurrences of the SAME (signature, output_digest) pair within
    /// `window` required to conclude no progress.
    threshold: usize,
}

impl Default for RepeatedOutputProgressStrategy {
    fn default() -> Self {
        Self {
            window: 32,
            threshold: 8,
        }
    }
}

impl RepeatedOutputProgressStrategy {
    /// Threshold accessor for callers that must avoid duplicating the
    /// literal (e.g. executor tests asserting recovery-warning counts).
    pub(crate) fn threshold(&self) -> usize {
        self.threshold
    }

    /// Count of the most common (signature, output_digest) pair within the
    /// trailing `window` of completed capability-call outputs.
    pub(crate) fn dominant_repeated_output_count<const N: usize>(
        &self,
        ring: &BoundedRing<CapabilityOutputObservation, N>,
    ) -> usize {
        ring.most_common_count_in(self.window)
    }

    /// Whether output repetition in `ring` constitutes no progress.
    pub(crate) fn is_no_progress<const N: usize>(
        &self,
        ring: &BoundedRing<CapabilityOutputObservation, N>,
    ) -> bool {
        self.dominant_repeated_output_count(ring) >= self.threshold
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::ids::CapabilityId;
    use ironclaw_loop_contracts::ContentDigest;
    use serde_json::json;

    use super::*;
    use crate::state::CapabilityCallSignature;

    fn observation(id: &str, arg: i64, digest: u64) -> CapabilityOutputObservation {
        CapabilityOutputObservation {
            signature: CapabilityCallSignature::from_call(
                CapabilityId::new(id).expect("valid"),
                &json!({ "x": arg }),
            )
            .expect("valid call signature"),
            output_digest: ContentDigest(digest),
        }
    }

    #[test]
    fn dominant_repeated_output_reaching_threshold_is_no_progress() {
        let strategy = RepeatedOutputProgressStrategy::default();
        let mut ring: BoundedRing<CapabilityOutputObservation, 64> = BoundedRing::new();
        for i in 0..24 {
            ring.push(observation("demo.filler", i, 1_000 + i as u64));
        }
        for _ in 0..strategy.threshold() {
            ring.push(observation("demo.echo", 1, 7));
        }

        assert_eq!(strategy.dominant_repeated_output_count(&ring), 8);
        assert!(strategy.is_no_progress(&ring));
    }

    #[test]
    fn same_signature_with_changing_output_never_reaches_no_progress() {
        let strategy = RepeatedOutputProgressStrategy::default();
        let mut ring: BoundedRing<CapabilityOutputObservation, 64> = BoundedRing::new();
        for i in 0..strategy.threshold() {
            ring.push(observation("cargo.test", 1, i as u64));
        }

        assert!(!strategy.is_no_progress(&ring));
    }

    #[test]
    fn seven_identical_outputs_stays_below_threshold() {
        let strategy = RepeatedOutputProgressStrategy::default();
        let mut ring: BoundedRing<CapabilityOutputObservation, 64> = BoundedRing::new();
        for _ in 0..(strategy.threshold() - 1) {
            ring.push(observation("demo.echo", 1, 9));
        }

        assert_eq!(strategy.dominant_repeated_output_count(&ring), 7);
        assert!(!strategy.is_no_progress(&ring));
    }

    #[test]
    fn empty_output_digest_ring_is_never_no_progress() {
        let strategy = RepeatedOutputProgressStrategy::default();
        let ring: BoundedRing<CapabilityOutputObservation, 64> = BoundedRing::new();

        assert_eq!(strategy.dominant_repeated_output_count(&ring), 0);
        assert!(!strategy.is_no_progress(&ring));
    }
}
