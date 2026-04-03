//! Gate pipeline — ordered evaluation of multiple [`ExecutionGate`]s.
//!
//! Gates are sorted by priority at construction time. The first gate to
//! return [`GateDecision::Pause`] or [`GateDecision::Deny`] wins.
//! If all gates return [`GateDecision::Allow`], execution proceeds.
//!
//! Gate implementations must not panic. A panicking gate will propagate
//! the panic to the caller (async `catch_unwind` is not used because
//! the gate evaluation borrows non-`UnwindSafe` context).

use std::sync::Arc;

use super::{ExecutionGate, GateContext, GateDecision};

/// Ordered pipeline of execution gates.
pub struct GatePipeline {
    gates: Vec<Arc<dyn ExecutionGate>>,
}

impl GatePipeline {
    /// Build a pipeline from the given gates, sorted by priority (ascending).
    pub fn new(mut gates: Vec<Arc<dyn ExecutionGate>>) -> Self {
        gates.sort_by_key(|g| g.priority());
        Self { gates }
    }

    /// Build an empty pipeline that allows everything (useful in tests).
    pub fn allow_all() -> Self {
        Self { gates: Vec::new() }
    }

    /// Evaluate all gates in priority order. First `Pause` or `Deny` wins.
    ///
    /// Gate implementations must not panic — a panic propagates to the caller.
    pub async fn evaluate(&self, ctx: &GateContext<'_>) -> GateDecision {
        for gate in &self.gates {
            let decision = gate.evaluate(ctx).await;
            match decision {
                GateDecision::Allow => continue,
                GateDecision::Pause { .. } | GateDecision::Deny { .. } => {
                    tracing::debug!(
                        gate = gate.name(),
                        tool = %ctx.action_name,
                        "gate stopped execution"
                    );
                    return decision;
                }
            }
        }
        GateDecision::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gate::{ExecutionMode, ResumeKind};
    use crate::types::capability::{ActionDef, EffectType};
    use crate::types::thread::ThreadId;
    use std::collections::HashSet;

    // ── Test helpers ────────────────────────────────────────

    fn test_action_def() -> ActionDef {
        ActionDef {
            name: "test_tool".into(),
            description: "a test tool".into(),
            parameters_schema: serde_json::json!({}),
            effects: vec![EffectType::ReadLocal],
            requires_approval: false,
        }
    }

    fn test_ctx<'a>(
        action_def: &'a ActionDef,
        auto_approved: &'a HashSet<String>,
        params: &'a serde_json::Value,
    ) -> GateContext<'a> {
        GateContext {
            user_id: "user1",
            thread_id: ThreadId::new(),
            source_channel: "web",
            action_name: &action_def.name,
            call_id: "call_1",
            parameters: params,
            action_def,
            execution_mode: ExecutionMode::Interactive,
            auto_approved,
        }
    }

    struct StaticGate {
        name: &'static str,
        priority: u32,
        decision: GateDecision,
    }

    #[async_trait::async_trait]
    impl ExecutionGate for StaticGate {
        fn name(&self) -> &str {
            self.name
        }
        fn priority(&self) -> u32 {
            self.priority
        }
        async fn evaluate(&self, _ctx: &GateContext<'_>) -> GateDecision {
            self.decision.clone()
        }
    }

    // ── Tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn test_all_allow_passes() {
        let pipeline = GatePipeline::new(vec![
            Arc::new(StaticGate {
                name: "a",
                priority: 10,
                decision: GateDecision::Allow,
            }),
            Arc::new(StaticGate {
                name: "b",
                priority: 20,
                decision: GateDecision::Allow,
            }),
        ]);
        let ad = test_action_def();
        let auto = HashSet::new();
        let params = serde_json::json!({});
        let ctx = test_ctx(&ad, &auto, &params);
        assert!(matches!(pipeline.evaluate(&ctx).await, GateDecision::Allow));
    }

    #[tokio::test]
    async fn test_first_deny_wins() {
        let pipeline = GatePipeline::new(vec![
            Arc::new(StaticGate {
                name: "allow",
                priority: 10,
                decision: GateDecision::Allow,
            }),
            Arc::new(StaticGate {
                name: "deny",
                priority: 20,
                decision: GateDecision::Deny {
                    reason: "blocked".into(),
                },
            }),
            Arc::new(StaticGate {
                name: "allow2",
                priority: 30,
                decision: GateDecision::Allow,
            }),
        ]);
        let ad = test_action_def();
        let auto = HashSet::new();
        let params = serde_json::json!({});
        let ctx = test_ctx(&ad, &auto, &params);
        assert!(matches!(
            pipeline.evaluate(&ctx).await,
            GateDecision::Deny { .. }
        ));
    }

    #[tokio::test]
    async fn test_first_pause_wins_over_later_deny() {
        let pipeline = GatePipeline::new(vec![
            Arc::new(StaticGate {
                name: "allow",
                priority: 10,
                decision: GateDecision::Allow,
            }),
            Arc::new(StaticGate {
                name: "pause",
                priority: 20,
                decision: GateDecision::Pause {
                    reason: "needs approval".into(),
                    resume_kind: ResumeKind::Approval { allow_always: true },
                },
            }),
            Arc::new(StaticGate {
                name: "deny",
                priority: 30,
                decision: GateDecision::Deny {
                    reason: "would deny".into(),
                },
            }),
        ]);
        let ad = test_action_def();
        let auto = HashSet::new();
        let params = serde_json::json!({});
        let ctx = test_ctx(&ad, &auto, &params);
        assert!(matches!(
            pipeline.evaluate(&ctx).await,
            GateDecision::Pause { .. }
        ));
    }

    #[tokio::test]
    async fn test_empty_pipeline_allows() {
        let pipeline = GatePipeline::allow_all();
        let ad = test_action_def();
        let auto = HashSet::new();
        let params = serde_json::json!({});
        let ctx = test_ctx(&ad, &auto, &params);
        assert!(matches!(pipeline.evaluate(&ctx).await, GateDecision::Allow));
    }

    #[tokio::test]
    async fn test_priority_ordering() {
        // Insert gates in reverse priority order — pipeline should still
        // evaluate the lower-priority (deny) gate first.
        let pipeline = GatePipeline::new(vec![
            Arc::new(StaticGate {
                name: "pause_high",
                priority: 200,
                decision: GateDecision::Pause {
                    reason: "late pause".into(),
                    resume_kind: ResumeKind::Approval {
                        allow_always: false,
                    },
                },
            }),
            Arc::new(StaticGate {
                name: "deny_low",
                priority: 10,
                decision: GateDecision::Deny {
                    reason: "early deny".into(),
                },
            }),
        ]);
        let ad = test_action_def();
        let auto = HashSet::new();
        let params = serde_json::json!({});
        let ctx = test_ctx(&ad, &auto, &params);
        match pipeline.evaluate(&ctx).await {
            GateDecision::Deny { reason } => assert_eq!(reason, "early deny"),
            other => panic!("Expected Deny, got {other:?}"),
        }
    }
}
