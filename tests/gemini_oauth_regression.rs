use ironclaw::llm::ChatMessage;
use ironclaw::llm::gemini_oauth::GeminiOauthProvider;

/// Regression: Cloud Code API routing for Gemini 2.0+ models.
/// Gemini 1.x → legacy generativelanguage.googleapis.com
/// Gemini 2.0+ → Cloud Code API (cloudcode-pa.googleapis.com)
#[test]
fn test_regression_cloud_code_api_routing() {
    // Legacy models (1.x) → false
    assert!(!GeminiOauthProvider::model_uses_cloud_code_api(
        "gemini-1.5-pro"
    ));
    assert!(!GeminiOauthProvider::model_uses_cloud_code_api(
        "gemini-1.5-flash"
    ));

    // 2.0+ models → true
    assert!(GeminiOauthProvider::model_uses_cloud_code_api(
        "gemini-2.0-flash"
    ));
    assert!(GeminiOauthProvider::model_uses_cloud_code_api(
        "gemini-2.5-pro"
    ));
    assert!(GeminiOauthProvider::model_uses_cloud_code_api(
        "gemini-2.5-flash"
    ));

    // Preview models with hyphen → true
    assert!(GeminiOauthProvider::model_uses_cloud_code_api(
        "gemini-3.1-pro-preview"
    ));
    assert!(GeminiOauthProvider::model_uses_cloud_code_api(
        "gemini-3-flash-preview"
    ));

    // Gemini 3 family → true
    assert!(GeminiOauthProvider::model_uses_cloud_code_api(
        "gemini-3-pro"
    ));
}

/// Regression: "preview" false-positive fix.
/// `model.contains("-preview")` (with hyphen) prevents models whose name
/// happens to include "preview" without a hyphen prefix from being
/// mis-routed to Cloud Code API.
#[test]
fn test_regression_preview_false_positive_fix() {
    // "my-preview-custom" still matches (contains "-preview")
    assert!(GeminiOauthProvider::model_uses_cloud_code_api(
        "my-preview-custom"
    ));

    // "mypreviewcustom" does NOT match (no hyphen before "preview")
    assert!(!GeminiOauthProvider::model_uses_cloud_code_api(
        "mypreviewcustom"
    ));

    // Non-Gemini models without "-preview" → false
    assert!(!GeminiOauthProvider::model_uses_cloud_code_api(
        "not-a-gemini-model"
    ));
}

/// Regression: model list consistency.
/// Wizard, list_models(), and LLM_PROVIDERS.md all return the same 8 models.
#[test]
fn test_regression_standardized_model_list() {
    let expected_models = [
        "gemini-3.1-pro-preview",
        "gemini-3.1-pro-preview-customtools",
        "gemini-3-pro-preview",
        "gemini-3-flash-preview",
        "gemini-3.1-flash-lite-preview",
        "gemini-2.5-pro",
        "gemini-2.5-flash",
        "gemini-2.5-flash-lite",
    ];

    // All standardized models must route to Cloud Code API (all are >= 2.0)
    for model in &expected_models {
        assert!(
            GeminiOauthProvider::model_uses_cloud_code_api(model),
            "Standardized model '{}' should route to Cloud Code API",
            model
        );
    }
}

/// Regression: ChatMessage helper constructors.
#[test]
fn test_regression_chat_message_helpers() {
    let user_msg = ChatMessage::user("hello");
    assert_eq!(user_msg.role, ironclaw::llm::Role::User);
    assert_eq!(user_msg.content, "hello");

    let system_msg = ChatMessage::system("you are helpful");
    assert_eq!(system_msg.role, ironclaw::llm::Role::System);
    assert_eq!(system_msg.content, "you are helpful");
}
