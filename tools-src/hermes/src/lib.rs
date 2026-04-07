//! Hermes WASM Tool for IronClaw.
//!
//! Provides access to the Hermes reasoning engine — NousResearch multi-model
//! AI for deep reasoning, code generation, and analysis tasks.
//!
//! # Capabilities Required
//!
//! - HTTP: `hermes-agent-production-61e5.up.railway.app/v1/*` (GET, POST)
//! - Secrets: `HERMES_API_KEY` (injected automatically)
//!
//! # Supported Actions
//!
//! - `hermes_chat`: General reasoning and conversation
//! - `hermes_analyze`: Structured analysis with task-specific prompts
//! - `hermes_code`: Code generation with language-specific optimization
//! - `hermes_models`: List available models

use serde::Deserialize;

// Generate bindings from the host WIT interface.
// This creates the `bindings` module with types and traits.
wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

struct HermesTool;

const BASE_URL: &str = "https://hermes-agent-production-61e5.up.railway.app/v1";
const DEFAULT_MODEL: &str = "NousResearch/Hermes-3-Llama-3.1-8B";

#[derive(Debug, Deserialize)]
#[serde(tag = "action")]
enum HermesAction {
    #[serde(rename = "hermes_chat")]
    Chat {
        message: String,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        system: Option<String>,
        #[serde(default)]
        temperature: Option<f64>,
    },
    #[serde(rename = "hermes_analyze")]
    Analyze {
        content: String,
        task: String,
        #[serde(default)]
        model: Option<String>,
    },
    #[serde(rename = "hermes_code")]
    Code {
        prompt: String,
        #[serde(default)]
        language: Option<String>,
        #[serde(default)]
        model: Option<String>,
    },
    #[serde(rename = "hermes_models")]
    Models,
}

impl exports::near::agent::tool::Guest for HermesTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params) {
            Ok(result) => exports::near::agent::tool::Response {
                output: Some(result),
                error: None,
            },
            Err(e) => exports::near::agent::tool::Response {
                output: None,
                error: Some(e),
            },
        }
    }

    fn schema() -> String {
        r#"{
            "type": "object",
            "required": ["action"],
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["hermes_chat", "hermes_analyze", "hermes_code", "hermes_models"],
                    "description": "The Hermes operation to perform"
                },
                "message": {
                    "type": "string",
                    "description": "The message or question to send to Hermes. Required for: hermes_chat"
                },
                "model": {
                    "type": "string",
                    "description": "Model to use (default: NousResearch/Hermes-3-Llama-3.1-8B). Used by: hermes_chat, hermes_analyze, hermes_code"
                },
                "system": {
                    "type": "string",
                    "description": "Custom system prompt. Used by: hermes_chat"
                },
                "temperature": {
                    "type": "number",
                    "description": "Sampling temperature 0-2 (default: 0.7). Used by: hermes_chat"
                },
                "content": {
                    "type": "string",
                    "description": "The content to analyze. Required for: hermes_analyze"
                },
                "task": {
                    "type": "string",
                    "description": "The analysis task (e.g., summarize, find bugs, evaluate architecture). Required for: hermes_analyze"
                },
                "prompt": {
                    "type": "string",
                    "description": "Description of the code to generate. Required for: hermes_code"
                },
                "language": {
                    "type": "string",
                    "description": "Target programming language (default: rust). Used by: hermes_code"
                }
            }
        }"#
        .to_string()
    }

    fn description() -> String {
        "Hermes reasoning engine — NousResearch multi-model AI for deep reasoning, \
         code generation, and analysis tasks. Requires a HERMES_API_KEY secret."
            .to_string()
    }
}

fn execute_inner(params: &str) -> Result<String, String> {
    if !crate::near::agent::host::secret_exists("HERMES_API_KEY") {
        return Err(
            "HERMES_API_KEY not configured. Set the secret to authenticate with the Hermes API."
                .to_string(),
        );
    }

    let action: HermesAction =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {}", e))?;

    match action {
        HermesAction::Chat {
            message,
            model,
            system,
            temperature,
        } => hermes_chat(
            &message,
            model.as_deref().unwrap_or(DEFAULT_MODEL),
            system
                .as_deref()
                .unwrap_or("You are Hermes, a deep reasoning AI assistant."),
            temperature.unwrap_or(0.7),
        ),
        HermesAction::Analyze {
            content,
            task,
            model,
        } => {
            let system_prompt = format!(
                "You are Hermes, an expert analyst. Your task: {}. \
                 Be thorough, structured, and precise.",
                task
            );
            hermes_chat(
                &content,
                model.as_deref().unwrap_or(DEFAULT_MODEL),
                &system_prompt,
                0.3,
            )
        }
        HermesAction::Code {
            prompt,
            language,
            model,
        } => {
            let lang = language.as_deref().unwrap_or("rust");
            let system_prompt = format!(
                "You are Hermes, an expert {} programmer. Write clean, \
                 production-quality code. Return only the code with brief comments.",
                lang
            );
            hermes_chat(
                &prompt,
                model.as_deref().unwrap_or(DEFAULT_MODEL),
                &system_prompt,
                0.2,
            )
        }
        HermesAction::Models => hermes_models(),
    }
}

/// Make an HTTP request to the Hermes API.
fn hermes_api_call(method: &str, endpoint: &str, body: Option<&str>) -> Result<String, String> {
    let url = format!("{}/{}", BASE_URL, endpoint);

    let headers = if body.is_some() {
        r#"{"Content-Type": "application/json"}"#
    } else {
        "{}"
    };

    let body_bytes = body.map(|b| b.as_bytes().to_vec());

    crate::near::agent::host::log(
        crate::near::agent::host::LogLevel::Debug,
        &format!("Hermes API: {} {}", method, endpoint),
    );

    let response =
        crate::near::agent::host::http_request(method, &url, headers, body_bytes.as_deref(), None)?;

    if response.status < 200 || response.status >= 300 {
        return Err(format!(
            "Hermes API returned status {}: {}",
            response.status,
            String::from_utf8_lossy(&response.body)
        ));
    }

    String::from_utf8(response.body).map_err(|e| format!("Invalid UTF-8 in response: {}", e))
}

/// POST to /v1/chat/completions with the given parameters.
fn hermes_chat(
    message: &str,
    model: &str,
    system_prompt: &str,
    temperature: f64,
) -> Result<String, String> {
    let body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": message}
        ],
        "temperature": temperature,
        "max_tokens": 4096
    });

    let body_str = serde_json::to_string(&body).map_err(|e| e.to_string())?;

    crate::near::agent::host::log(
        crate::near::agent::host::LogLevel::Info,
        &format!("hermes_chat: model={} len={}", model, message.len()),
    );

    let response = hermes_api_call("POST", "chat/completions", Some(&body_str))?;

    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

    if let Some(content) = parsed["choices"][0]["message"]["content"].as_str() {
        Ok(serde_json::json!({
            "response": content,
            "model": parsed["model"],
            "usage": parsed["usage"]
        })
        .to_string())
    } else {
        Ok(response)
    }
}

/// GET /v1/models to list available models.
fn hermes_models() -> Result<String, String> {
    crate::near::agent::host::log(
        crate::near::agent::host::LogLevel::Info,
        "hermes_models: listing available models",
    );
    hermes_api_call("GET", "models", None)
}

// Export the tool implementation.
export!(HermesTool);
