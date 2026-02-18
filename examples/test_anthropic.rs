//! Quick smoke test for the Anthropic provider.
//!
//! Usage:
//!   # With API key:
//!   ANTHROPIC_API_KEY=sk-ant-... cargo run --example test_anthropic
//!
//!   # With OAuth token (from `claude setup-token`):
//!   CLAUDE_CODE_OAUTH_TOKEN=... cargo run --example test_anthropic

use ironclaw::config::{AnthropicAuth, AnthropicDirectConfig};
use ironclaw::llm::{AnthropicProvider, ChatMessage, CompletionRequest, LlmProvider};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter("ironclaw=debug")
        .init();

    // Resolve auth from environment
    let auth = if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        println!("Using API key auth");
        AnthropicAuth::ApiKey(secrecy::SecretString::from(key))
    } else if let Ok(token) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN") {
        println!("Using OAuth token auth (Max subscription)");
        AnthropicAuth::OAuthToken {
            access_token: secrecy::SecretString::from(token),
            refresh_token: None,
        }
    } else {
        eprintln!("Set ANTHROPIC_API_KEY or CLAUDE_CODE_OAUTH_TOKEN");
        std::process::exit(1);
    };

    let model =
        std::env::var("ANTHROPIC_MODEL").unwrap_or_else(|_| "claude-sonnet-4-20250514".to_string());

    println!("Model: {}", model);

    let config = AnthropicDirectConfig {
        auth,
        model,
        max_retries: 2,
    };

    let provider = AnthropicProvider::new(config);

    // Send a simple completion
    let request = CompletionRequest::new(vec![
        ChatMessage::system("You are a helpful assistant. Reply in one sentence."),
        ChatMessage::user("What is Rust's borrow checker?"),
    ])
    .with_max_tokens(100);

    println!("\nSending request...");

    match provider.complete(request).await {
        Ok(response) => {
            println!("\nResponse: {}", response.content);
            println!(
                "Tokens: {} in / {} out",
                response.input_tokens, response.output_tokens
            );
            println!("Finish reason: {:?}", response.finish_reason);

            let cost = provider.calculate_cost(response.input_tokens, response.output_tokens);
            println!("Cost: ${}", cost);
            println!("\nSmoke test PASSED");
        }
        Err(e) => {
            eprintln!("\nRequest FAILED: {}", e);
            std::process::exit(1);
        }
    }
}
