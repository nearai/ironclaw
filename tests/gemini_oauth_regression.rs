//! Regression coverage for `ironclaw_llm` symbols consumed by the binary.
//!
//! Cloud Code API routing for Gemini is covered by unit tests inside
//! `crates/ironclaw_llm/src/gemini_oauth.rs` (which can reach the now-private
//! `GeminiOauthProvider::model_uses_cloud_code_api`). External callers must
//! not import `ironclaw_llm::gemini_oauth` — that module is crate-private.
use ironclaw_llm::ChatMessage;

/// Regression: `ChatMessage` helper constructors.
#[test]
fn test_regression_chat_message_helpers() {
    let user_msg = ChatMessage::user("hello");
    assert_eq!(user_msg.role, ironclaw_llm::Role::User);
    assert_eq!(user_msg.content, "hello");

    let system_msg = ChatMessage::system("you are helpful");
    assert_eq!(system_msg.role, ironclaw_llm::Role::System);
    assert_eq!(system_msg.content, "you are helpful");
}
