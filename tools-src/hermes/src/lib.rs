wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "wit/sandboxed-tool.wit",
});

use exports::ironclaw::tool::guest::Guest;
use ironclaw::tool::host;

struct HermesTool;

impl Guest for HermesTool {
    fn call(name: String, input: String) -> String {
        let base_url = "https://hermes-agent-production-61e5.up.railway.app/v1";
        let headers = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Authorization".to_string(), "Bearer {HERMES_API_KEY}".to_string()),
        ];

        match name.as_str() {
            "hermes_chat" => {
                let parsed: serde_json::Value = serde_json::from_str(&input).unwrap_or_default();
                let message = parsed["message"].as_str().unwrap_or("");
                let model = parsed["model"].as_str()
                    .unwrap_or("NousResearch/Hermes-3-Llama-3.1-8B");
                let system = parsed["system"].as_str()
                    .unwrap_or("You are Hermes, a deep reasoning AI assistant.");
                let temperature = parsed["temperature"].as_f64().unwrap_or(0.7);

                let url = format!("{}/chat/completions", base_url);
                let body = serde_json::json!({
                    "model": model,
                    "messages": [
                        {"role": "system", "content": system},
                        {"role": "user", "content": message}
                    ],
                    "temperature": temperature,
                    "max_tokens": 4096
                }).to_string();

                host::log(host::LogLevel::Info, &format!("hermes_chat: model={} len={}", model, message.len()));
                match host::http_request("POST", &url, &headers, Some(&body)) {
                    Ok(response) => {
                        if let Ok(resp) = serde_json::from_str::<serde_json::Value>(&response) {
                            if let Some(content) = resp["choices"][0]["message"]["content"].as_str() {
                                return serde_json::json!({
                                    "response": content,
                                    "model": resp["model"],
                                    "usage": resp["usage"]
                                }).to_string();
                            }
                        }
                        response
                    }
                    Err(e) => serde_json::json!({"error": e}).to_string(),
                }
            }

            "hermes_analyze" => {
                let parsed: serde_json::Value = serde_json::from_str(&input).unwrap_or_default();
                let content = parsed["content"].as_str().unwrap_or("");
                let task = parsed["task"].as_str().unwrap_or("analyze");
                let model = parsed["model"].as_str()
                    .unwrap_or("NousResearch/Hermes-3-Llama-3.1-8B");

                let url = format!("{}/chat/completions", base_url);
                let system_prompt = format!(
                    "You are Hermes, an expert analyst. Your task: {}. Be thorough, structured, and precise.", task
                );
                let body = serde_json::json!({
                    "model": model,
                    "messages": [
                        {"role": "system", "content": system_prompt},
                        {"role": "user", "content": content}
                    ],
                    "temperature": 0.3,
                    "max_tokens": 4096
                }).to_string();

                host::log(host::LogLevel::Info, &format!("hermes_analyze: task={}", task));
                match host::http_request("POST", &url, &headers, Some(&body)) {
                    Ok(response) => {
                        if let Ok(resp) = serde_json::from_str::<serde_json::Value>(&response) {
                            if let Some(content) = resp["choices"][0]["message"]["content"].as_str() {
                                return serde_json::json!({
                                    "analysis": content,
                                    "task": task,
                                    "model": resp["model"],
                                    "usage": resp["usage"]
                                }).to_string();
                            }
                        }
                        response
                    }
                    Err(e) => serde_json::json!({"error": e}).to_string(),
                }
            }

            "hermes_code" => {
                let parsed: serde_json::Value = serde_json::from_str(&input).unwrap_or_default();
                let prompt = parsed["prompt"].as_str().unwrap_or("");
                let language = parsed["language"].as_str().unwrap_or("rust");
                let model = parsed["model"].as_str()
                    .unwrap_or("NousResearch/Hermes-3-Llama-3.1-8B");

                let url = format!("{}/chat/completions", base_url);
                let system_prompt = format!(
                    "You are Hermes, an expert {} programmer. Write clean, production-quality code. Return only the code with brief comments.", language
                );
                let body = serde_json::json!({
                    "model": model,
                    "messages": [
                        {"role": "system", "content": system_prompt},
                        {"role": "user", "content": prompt}
                    ],
                    "temperature": 0.2,
                    "max_tokens": 4096
                }).to_string();

                host::log(host::LogLevel::Info, &format!("hermes_code: lang={}", language));
                match host::http_request("POST", &url, &headers, Some(&body)) {
                    Ok(response) => {
                        if let Ok(resp) = serde_json::from_str::<serde_json::Value>(&response) {
                            if let Some(content) = resp["choices"][0]["message"]["content"].as_str() {
                                return serde_json::json!({
                                    "code": content,
                                    "language": language,
                                    "model": resp["model"],
                                    "usage": resp["usage"]
                                }).to_string();
                            }
                        }
                        response
                    }
                    Err(e) => serde_json::json!({"error": e}).to_string(),
                }
            }

            "hermes_models" => {
                let url = format!("{}/models", base_url);
                host::log(host::LogLevel::Info, "hermes_models: listing available models");
                match host::http_request("GET", &url, &headers, None) {
                    Ok(response) => response,
                    Err(e) => serde_json::json!({"error": e}).to_string(),
                }
            }

            _ => serde_json::json!({"error": format!("unknown function: {}", name)}).to_string(),
        }
    }
}

export!(HermesTool);
