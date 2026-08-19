//! Product-surface-neutral suggestion operation declarations.

use crate::descriptors::{ProductSurfaceCommandDescriptor, ProductView};
use crate::inbound_requests::{
    RebornSuggestionDismissRequest, RebornSuggestionStartRequest, RebornSuggestionsGenerateRequest,
    RebornSuggestionsListRequest,
};
use crate::product_wire::{
    RebornSuggestionDismissResponse, RebornSuggestionStartResponse, RebornSuggestionsResponse,
};

pub const SUGGESTIONS_LIST_VIEW: ProductView<
    RebornSuggestionsListRequest,
    RebornSuggestionsResponse,
> = ProductView::unpaginated("suggestions.list");

pub const SUGGESTIONS_GENERATE_COMMAND_ID: &str = "suggestions.generate";
pub const SUGGESTIONS_GENERATE_COMMAND: ProductSurfaceCommandDescriptor<
    RebornSuggestionsGenerateRequest,
    RebornSuggestionsResponse,
> = ProductSurfaceCommandDescriptor::new(SUGGESTIONS_GENERATE_COMMAND_ID);

pub const SUGGESTION_START_COMMAND_ID: &str = "suggestion.start";
pub const SUGGESTION_START_COMMAND: ProductSurfaceCommandDescriptor<
    RebornSuggestionStartRequest,
    RebornSuggestionStartResponse,
> = ProductSurfaceCommandDescriptor::new(SUGGESTION_START_COMMAND_ID);

pub const SUGGESTION_DISMISS_COMMAND_ID: &str = "suggestion.dismiss";
pub const SUGGESTION_DISMISS_COMMAND: ProductSurfaceCommandDescriptor<
    RebornSuggestionDismissRequest,
    RebornSuggestionDismissResponse,
> = ProductSurfaceCommandDescriptor::new(SUGGESTION_DISMISS_COMMAND_ID);

#[cfg(test)]
mod tests {
    use crate::inbound_requests::{RebornSuggestionsGenerateRequest, RebornSuggestionsListRequest};
    use crate::product_wire::{RebornSuggestionGenerationStatus, RebornSuggestionsResponse};

    #[test]
    fn generate_requires_a_client_action_id_and_list_is_empty_input() {
        let generated = serde_json::to_value(RebornSuggestionsGenerateRequest {
            client_action_id: "suggestions-action-1".to_string(),
        })
        .expect("generate request serializes");
        assert_eq!(
            generated,
            serde_json::json!({"client_action_id": "suggestions-action-1"})
        );

        let list = serde_json::to_value(RebornSuggestionsListRequest::default())
            .expect("list request serializes");
        assert_eq!(list, serde_json::json!({}));

        let response = RebornSuggestionsResponse {
            status: RebornSuggestionGenerationStatus::Generating,
            generation_id: Some("generation-1".to_string()),
            retry_after_seconds: Some(1),
            suggestions: Vec::new(),
        };
        assert_eq!(
            serde_json::to_value(response).expect("response serializes"),
            serde_json::json!({
                "status": "generating",
                "generation_id": "generation-1",
                "retry_after_seconds": 1,
                "suggestions": []
            })
        );
    }
}
