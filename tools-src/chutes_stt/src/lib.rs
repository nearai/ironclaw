//! Chutes.ai Speech-to-Text WASM Tool
//! 
//! This tool transcribes audio files using Chutes.ai Whisper API.

use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use serde::{Deserialize, Serialize};
use js_sys::Uint8Array;
use web_sys::console;

/// Input schema for STT transcription
#[derive(Debug, Deserialize)]
pub struct SttInput {
    /// Path to audio file to transcribe
    pub audio_path: String,
    /// Output path for transcription text
    pub output_path: String,
    /// MIME type of audio (e.g., "audio/ogg", "audio/wav")
    #[serde(default = "default_mime_type")]
    pub mime_type: String,
    /// Language code (optional, auto-detect if not provided)
    pub language: Option<String>,
    /// Model to use (default: "openai/whisper-large-v3-turbo")
    #[serde(default = "default_model")]
    pub model: String,
}

fn default_mime_type() -> String { "audio/wav".to_string() }
fn default_model() -> String { "openai/whisper-large-v3-turbo".to_string() }

/// Output schema for STT transcription
#[derive(Debug, Serialize)]
pub struct SttOutput {
    /// Transcribed text
    pub transcription: String,
    /// Confidence score (0.0 to 1.0)
    pub confidence: Option<f32>,
    /// Detected language code
    pub language_detected: Option<String>,
    /// Audio duration in milliseconds
    pub duration_ms: Option<u32>,
}

/// Error types for STT operations
#[derive(Debug, thiserror::Error)]
pub enum SttError {
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    #[error("Path not in allowed scope: {0}")]
    PathNotAllowed(String),
    #[error("Failed to read audio file: {0}")]
    ReadAudioError(String),
    #[error("Chutes API error: {0}")]
    ApiError(String),
    #[error("Failed to write output: {0}")]
    WriteError(String),
}

impl SttError {
    fn to_js_value(&self) -> JsValue {
        JsValue::from_str(&self.to_string())
    }
}

/// Validate that a path is within allowed scopes
fn validate_path_scope(path: &str, allowed_scopes: &[&str]) -> bool {
    allowed_scopes.iter().any(|scope| path.starts_with(scope))
}

/// Read audio file and convert to base64
async fn read_audio_file(path: &str) -> Result<String, SttError> {
    let path_js = JsValue::from_str(path);
    
    // Call host function to read file (provided by ironclaw runtime)
    let read_fn = js_sys::Function::new_no_args_with_src(
        "window.__ironclaw_read_file || (() => Promise.reject(new Error('File read not available')))"
    );
    
    let result = read_fn
        .call1(&JsValue::NULL, &path_js)
        .map_err(|e| SttError::ReadAudioError(format!("JS error: {:?}", e)))?;
    
    let promise = JsFuture::from(result);
    let buffer = promise
        .await
        .map_err(|e| SttError::ReadAudioError(format!("Await error: {:?}", e)))?;
    
    let uint8_array = buffer.dyn_into::<Uint8Array>()
        .map_err(|_| SttError::ReadAudioError("Not a byte array"))?;
    
    let base64_str = base64::encode(uint8_array.to_vec());
    Ok(base64_str)
}

/// Call Chutes.ai Whisper API
async fn call_chutes_stt_api(
    audio_base64: &str,
    mime_type: &str,
    model: &str,
    language: Option<&str>,
) -> Result<SttOutput, SttError> {
    let api_key = std::env::var("CHUTES_API_KEY")
        .map_err(|_| SttError::ApiError("CHUTES_API_KEY not set".to_string()))?;
    
    // Build request payload
    let mut payload = serde_json::json!({
        "model": model,
        "file": audio_base64,
        "response_format": "json",
    });
    
    if let Some(lang) = language {
        payload["language"] = serde_json::json!(lang);
    }
    
    // Make API request
    let client = reqwest::Client::new();
    let response = client
        .post("https://llm.chutes.ai/v1/audio/transcriptions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| SttError::ApiError(format!("Request failed: {}", e)))?;
    
    if !response.status().is_success() {
        let status = response.status();
        let error_text = response.text().await.unwrap_or_default();
        return Err(SttError::ApiError(format!("API error {}: {}", status, error_text)));
    }
    
    // Parse response
    let json: serde_json::Value = response.json()
        .await
        .map_err(|e| SttError::ApiError(format!("Failed to parse response: {}", e)))?;
    
    let transcription = json["text"]
        .as_str()
        .unwrap_or("")
        .to_string();
    
    let confidence = json["confidence"].as_f64().map(|f| f as f32);
    let language_detected = json["language"].as_str().map(|s| s.to_string());
    
    Ok(SttOutput {
        transcription,
        confidence,
        language_detected,
        duration_ms: None,
    })
}

/// Write transcription text to output file
fn write_output_file(path: &str, text: &str) -> Result<(), SttError> {
    let path_js = JsValue::from_str(path);
    let text_js = JsValue::from_str(text);
    
    let write_fn = js_sys::Function::new_no_args_with_src(
        "window.__ironclaw_write_file || (() => Promise.reject(new Error('File write not available')))"
    );
    
    let result = write_fn
        .call2(&JsValue::NULL, &path_js, &text_js)
        .map_err(|e| SttError::WriteError(format!("JS error: {:?}", e)))?;
    
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = JsFuture::from(result).await {
            console::error_1(&JsValue::from_str(&format!("Write failed: {:?}", e)));
        }
    });
    
    Ok(())
}

/// Fallback transcription with whisper-large-v3
async fn transcribe_fallback(input: &SttInput) -> Result<SttOutput, SttError> {
    console::log_1(&JsValue::from_str("Using fallback STT with whisper-large-v3"));
    
    let audio_base64 = read_audio_file(&input.audio_path).await?;
    
    let result = call_chutes_stt_api(
        &audio_base64,
        &input.mime_type,
        "openai/whisper-large-v3",
        input.language.as_deref(),
    ).await?;
    
    write_output_file(&input.output_path, &result.transcription)?;
    
    Ok(result)
}

/// Main transcription entry point
#[wasm_bindgen]
pub async fn transcribe(input_json: String) -> Result<JsValue, JsValue> {
    console::log_1(&JsValue::from_str("chutes_stt: Starting transcription"));
    
    // Parse input
    let input: SttInput = serde_json::from_str(&input_json)
        .map_err(|e| SttError::InvalidInput(format!("Invalid JSON: {}", e)))?;
    
    // Validate paths
    let read_scopes = ["/agents/mentor/", "/tmp/stt_input/", "/tmp/"];
    let write_scopes = ["/tmp/stt_output/", "/tmp/stt_cache/", "/tmp/"];
    
    if !validate_path_scope(&input.audio_path, &read_scopes) {
        return Err(SttError::PathNotAllowed(format!(
            "Audio path {} not in allowed scopes: {:?}",
            input.audio_path, read_scopes
        )).to_js_value());
    }
    
    if !validate_path_scope(&input.output_path, &write_scopes) {
        return Err(SttError::PathNotAllowed(format!(
            "Output path {} not in allowed scopes: {:?}",
            input.output_path, write_scopes
        )).to_js_value());
    }
    
    // Read audio file
    console::log_1(&JsValue::from_str(&format!("Reading audio file: {}", input.audio_path)));
    let audio_base64 = read_audio_file(&input.audio_path).await?;
    
    // Call Chutes API
    console::log_1(&JsValue::from_str(&format!(
        "Calling Chutes API with model: {}",
        input.model
    )));
    
    let result = call_chutes_stt_api(
        &audio_base64,
        &input.mime_type,
        &input.model,
        input.language.as_deref(),
    ).await?;
    
    // Write output
    write_output_file(&input.output_path, &result.transcription)?;
    
    console::log_1(&JsValue::from_str("chutes_stt: Transcription complete"));
    
    Ok(serde_wasm_bindgen::to_value(&result)?)
}
