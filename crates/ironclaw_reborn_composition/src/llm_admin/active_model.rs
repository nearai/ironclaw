//! Live active-model reader backing default-model run cost pricing.
//!
//! A WebChat v2 run submitted without an explicit `model` carries no
//! `resolved_model_route`, so the product service has no model id to price its
//! captured token usage against. [`ProviderActiveModelReader`] closes that gap
//! by reading the runtime's hot-swappable primary provider handle: for a
//! default (unrouted) run, the provider's `active_model_name()` is exactly the
//! model that ran, and it tracks operator model swaps because the handle is the
//! same [`SwappableLlmProvider`](ironclaw_llm::SwappableLlmProvider) the model
//! gateway drives.

use std::sync::Arc;

use ironclaw_llm::LlmProvider;
use ironclaw_product::ActiveModelReader;

/// [`ActiveModelReader`] over the runtime's live primary provider handle.
pub(crate) struct ProviderActiveModelReader {
    provider: Arc<dyn LlmProvider>,
}

impl ProviderActiveModelReader {
    pub(crate) fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self { provider }
    }
}

impl ActiveModelReader for ProviderActiveModelReader {
    fn active_model_id(&self) -> Option<String> {
        let name = self.provider.active_model_name();
        let trimmed = name.trim();
        // A cold-boot placeholder reports `unconfigured`, and some providers can
        // report a non-concrete alias; treat those (and empty) as "no concrete
        // model" so a run is reported without cost rather than mispriced. A run
        // that actually produced token usage always ran on a real provider, so
        // the common case yields a concrete model id here.
        if trimmed.is_empty()
            || trimmed.eq_ignore_ascii_case("default")
            || trimmed.eq_ignore_ascii_case("unconfigured")
        {
            None
        } else {
            Some(trimmed.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ironclaw_llm::{
        CompletionRequest, CompletionResponse, LlmError, ToolCompletionRequest,
        ToolCompletionResponse,
    };
    use rust_decimal::Decimal;

    struct FixedModelProvider {
        active_model: String,
    }

    #[async_trait]
    impl LlmProvider for FixedModelProvider {
        fn model_name(&self) -> &str {
            "fixed"
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        fn active_model_name(&self) -> String {
            self.active_model.clone()
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            unreachable!("pricing reader never issues completions")
        }

        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            unreachable!("pricing reader never issues completions")
        }
    }

    fn reader(active_model: &str) -> ProviderActiveModelReader {
        ProviderActiveModelReader::new(Arc::new(FixedModelProvider {
            active_model: active_model.to_string(),
        }))
    }

    #[test]
    fn surfaces_concrete_active_model() {
        assert_eq!(
            reader("openai/gpt-4o").active_model_id().as_deref(),
            Some("openai/gpt-4o")
        );
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(
            reader("  gpt-4o  ").active_model_id().as_deref(),
            Some("gpt-4o")
        );
    }

    #[test]
    fn rejects_non_concrete_placeholders() {
        assert_eq!(reader("").active_model_id(), None);
        assert_eq!(reader("   ").active_model_id(), None);
        assert_eq!(reader("default").active_model_id(), None);
        assert_eq!(reader("DEFAULT").active_model_id(), None);
        assert_eq!(reader("unconfigured").active_model_id(), None);
    }
}
