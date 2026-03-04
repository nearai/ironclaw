//! Chutes.ai Text-to-Speech WASM Tool with Voice Cloning
//! 
//! This tool synthesizes speech from text using Chutes.ai API,
//! supporting zero-shot voice cloning from a reference audio sample.

use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use serde::{Deserialize, Serialize};
use js_sys::Uint8Array;
use web_sys::console;

/// Input schema for TTS synthesis
#[derive(Debug, Deserialize)]
pub struct TtsInput {
    /// Text to synthesize
    pub text: String,
    /// Path to reference audio for voice cloning (e.g., /agents/mentor/master-voice.wav)
    pub reference_audio_path: Option<String>,
    /// Output path for synthesized audio
    pub output_path: String,
    /// Model to use (default: "sesame/csm-1b")
    #[serde(default = "default_model")]
    pub model: String,
    /// Audio format (default: "wav")
    #[serde(default = "default_format")]
    pub format: String,
    /// Speech speed multiplier (default: 1.0)
    #[serde(default = "default_speed")]
    pub speed: f32,
}

fn default_model() -> String { "sesame/csm-1b".to_string() }
fn default_format() -> String { "wav".to_string() }
fn default_speed() -> f32 { 1.0 }

/// Output schema for TTS synthesis
#[derive(Debug, Serialize)]
pub struct TtsOutput {
    /// Path to generated audio file
    pub audio_path: String,
    /// Duration in milliseconds
    pub duration_ms: Option<u32>,
    /// Model used for synthesis
    pub model_used: String,
    /// Number of characters synthesized
    pub character_count: usize,
}

/// Error types for TTS operations
#[derive(Debug, thiserror::Error)]
pub enum TtsError {
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    #[error("Path not in allowed scope: {0}")]
    PathNotAllowed(String),
    #[error("Failed to read reference audio: {0}")]
    ReadAudioError(String),
    #[error("Chutes API error: {0}")]
    ApiError(String),
    #[error("Failed to write output: {0}")]
    WriteError(String),
}

impl TtsError {
    fn to_js_value(&self) -> JsValue {
        JsValue::from_str(&self.to_string())
    }
}

/// Validate that a path is within allowed scopes
fn validate_path_scope(path: &str, allowed_scopes: &[&str]) -> bool {
    allowed_scopes.iter().any(|scope| path.starts_with(scope))
}

/// Read audio file and convert to base64
async fn read_audio_file(path: &str) -> Result<String, TtsError> {
    // In WASM sandbox, use host-provided file reading
    let path_js = JsValue::from_str(path);
    
    // Call host function to read file (provided by ironclaw runtime)
    let read_promise = js_sys::Function::new_no_args_with_src(
        "window.__ironclaw_read_file || (() => Promise.reject(new Error('File read not available')))"
    )
    .call1(&JsValue::NULL, &path_js)
    .map_err(|e| TtsError::ReadAudioError(format!("JS error: {:?}", e)))?;
    
    let result = JsFuture::from(read_promise.as_f64().map(|_| read_promise).unwrap_or(read_promise))
        .await
        .map_err(|e| TtsError::ReadAudioError(format!("Await error: {:?}", e)))?;
    
    // Convert to base64
    let buffer = result.dyn_into::<Uint8Array>()
        .map_err(|_| TtsError::ReadAudioError("Not a byte array"))?;
    
    let base64_str = base64::encode(buffer.to_vec());
    Ok(base64_str)
}

/// Call Chutes.ai TTS API
async fn call_chutes_tts_api(
    text: &str,
    reference_audio_base64: Option<&str>,
    model: &str,
    format: &str,
    speed: f32,
) -> Result<Vec<u8>, TtsError> {
    let api_key = std::env::var("CHUTES_API_KEY")
        .map_err(|_| TtsError::ApiError("CHUTES_API_KEY not set".to_string()))?;
    
    // Build request payload
    let mut payload = serde_json::json!({
        "model": model,
        "input": text,
        "response_format": format,
        "speed": speed,
    });
    
    // Add voice cloning parameters if reference audio provided
    if let Some(audio_b64) = reference_audio_base64 {
        payload["voice"] = serde_json::json!({
            "type": "base64",
            "data": audio_b64,
        });
    }
    
    // Make API request
    let client = reqwest::Client::new();
    let response = client
        .post("https://llm.chutes.ai/v1/audio/speech")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| TtsError::ApiError(format!("Request failed: {}", e)))?;
    
    if !response.status().is_success() {
        let status = response.status();
        let error_text = response.text().await.unwrap_or_default();
        return Err(TtsError::ApiError(format!("API error {}: {}", status, error_text)));
    }
    
    let audio_bytes = response.bytes()
        .await
        .map_err(|e| TtsError::ApiError(format!("Failed to read response: {}", e)))?;
    
    Ok(audio_bytes.to_vec())
}

/// Fallback synthesis without voice cloning (uses default Kokoro voice)
async fn synthesize_fallback(input: &TtsInput) -> Result<TtsOutput, TtsError> {
    console::log_1(&JsValue::from_str("Using fallback TTS without voice cloning"));
    
    let audio_bytes = call_chutes_tts_api(
        &input.text,
        None,
        "hexgrad/Kokoro-82M",
        &input.format,
        input.speed,
    ).await?;
    
    // Write output file (via host function)
    write_output_file(&input.output_path, &audio_bytes)?;
    
    Ok(TtsOutput {
        audio_path: input.output_path.clone(),
        duration_ms: None,
        model_used: "hexgrad/Kokoro-82M".to_string(),
        character_count: input.text.len(),
    })
}

/// Write audio data to output file
fn write_output_file(path: &str, data: &[u8]) -> Result<(), TtsError> {
    let path_js = JsValue::from_str(path);
    let data_array = Uint8Array::from(data);
    
    let write_promise = js_sys::Function::new_no_args_with_src(
        "window.__ironclaw_write_file || (() => Promise.reject(new Error('File write not available')))"
    )
    .call2(&JsValue::NULL, &path_js, &data_array)
    .map_err(|e| TtsError::WriteError(format!("JS error: {:?}", e)))?;
    
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = JsFuture::from(write_promise).await {
            console::error_1(&JsValue::from_str(&format!("Write failed: {:?}", e)));
        }
    });
    
    Ok(())
}

/// Main synthesis entry point
#[wasm_bindgen]
pub async fn synthesize(input_json: String) -> Result<JsValue, JsValue> {
    console::log_1(&JsValue::from_str("chutes_tts: Starting synthesis"));
    
    // Parse input
    let input: TtsInput = serde_json::from_str(&input_json)
        .map_err(|e| TtsError::InvalidInput(format!("Invalid JSON: {}", e)))?;
    
    // Validate paths
    let read_scopes = ["/agents/mentor/", "/tmp/tts_input/"];
    let write_scopes = ["/tmp/tts_output/", "/tmp/tts_cache/", "/agents/mentor/checkpoints/"];
    
    if let Some(ref path) = input.reference_audio_path {
        if !validate_path_scope(path, &read_scopes) {
            return Err(TtsError::PathNotAllowed(format!(
                "Reference audio path {} not in allowed scopes: {:?}",
                path, read_scopes
            )).to_js_value());
        }
    }
    
    if !validate_path_scope(&input.output_path, &write_scopes) {
        return Err(TtsError::PathNotAllowed(format!(
            "Output path {} not in allowed scopes: {:?}",
            input.output_path, write_scopes
        )).to_js_value());
    }
    
    // Read reference audio if provided
    let reference_audio_base64 = if let Some(ref path) = input.reference_audio_path {
        console::log_1(&JsValue::from_str(&format!("Reading reference audio: {}", path)));
        Some(read_audio_file(path).await?)
    } else {
        console::log_1(&JsValue::from_str("No reference audio provided, using default voice"));
        None
    };
    
    // Call Chutes API
    console::log_1(&JsValue::from_str(&format!(
        "Calling Chutes API with model: {}",
        input.model
    )));
    
    let audio_bytes = call_chutes_tts_api(
        &input.text,
        reference_audio_base64.as_deref(),
        &input.model,
        &input.format,
        input.speed,
    ).await?;
    
    // Write output
    write_output_file(&input.output_path, &audio_bytes)?;
    
    console::log_1(&JsValue::from_str("chutes_tts: Synthesis complete"));
    
    let output = TtsOutput {
        audio_path: input.output_path.clone(),
        duration_ms: None,
        model_used: input.model.clone(),
        character_count: input.text.len(),
    };
    
    Ok(serde_wasm_bindgen::to_value(&output)?)
}
