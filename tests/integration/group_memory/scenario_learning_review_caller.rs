//! Caller-level learning coverage through the production-shaped turn sink.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use ironclaw_loop_host::learning_review::{LearningInferenceError, LearningInferencePort};
use ironclaw_memory::LearningReviewRecord;

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;

const REVIEW: &str = r#"{
    "memory": [{
        "kind": "preference",
        "content": "The user prefers concise updates",
        "source_message_indices": [0],
        "confidence_basis_points": 9000,
        "explicitness": "explicit",
        "tainted": false
    }],
    "skill": {
        "action": "skip",
        "reason": null,
        "source_message_indices": []
    }
}"#;

struct RecordingInference {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl LearningInferencePort for RecordingInference {
    async fn infer(&self, _system: &str, _user: &str) -> Result<String, LearningInferenceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(REVIEW.to_string())
    }
}

pub async fn enabled_persists_one_sealed_candidate() -> HarnessResult<()> {
    let calls = Arc::new(AtomicUsize::new(0));
    let inference = Arc::new(RecordingInference {
        calls: Arc::clone(&calls),
    });
    let group = RebornIntegrationGroup::builder()
        .with_learning_review_for_test(inference, true)
        .builtin_tools()
        .await?;
    let harness = group
        .thread("conv-learning-enabled")
        .script([RebornScriptedReply::text("turn complete")])
        .build()
        .await?;

    let run_id = harness.submit_turn("remember my update preference").await?;
    harness.assert_reply_contains("turn complete").await?;
    let records = harness.wait_for_learning_candidate_for_test().await?;
    harness.shutdown_learning_review_for_test().await?;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(records.len(), 1);
    let record: &LearningReviewRecord = &records[0];
    assert_eq!(record.run_id, run_id);
    assert_eq!(record.scope.tenant_id().as_str(), "tenant-itest");
    assert!(!record.scope.user_id().as_str().is_empty());
    assert_eq!(record.scope.agent_id().as_str(), "agent-itest");
    assert_eq!(
        record.scope.project_id().map(|id| id.as_str()),
        Some("project-itest")
    );
    assert!(record.review.memory[0].tainted);
    Ok(())
}

pub async fn disabled_does_not_infer_or_persist() -> HarnessResult<()> {
    let calls = Arc::new(AtomicUsize::new(0));
    let inference = Arc::new(RecordingInference {
        calls: Arc::clone(&calls),
    });
    let group = RebornIntegrationGroup::builder()
        .with_learning_review_for_test(inference, false)
        .builtin_tools()
        .await?;
    let harness = group
        .thread("conv-learning-disabled")
        .script([RebornScriptedReply::text("turn complete")])
        .build()
        .await?;

    harness.submit_turn("do not learn this").await?;
    harness.assert_reply_contains("turn complete").await?;
    harness.shutdown_learning_review_for_test().await?;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(harness.learning_candidates_for_test().await?.is_empty());
    Ok(())
}
