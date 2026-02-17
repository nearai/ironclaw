#![allow(unused_imports, unused_variables, dead_code)]
use anyhow::{Context, Result};
use clap::Parser;
#[cfg(target_arch = "aarch64")]
use rppal::gpio::{Gpio, Level, Mode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::{error, info, warn};

/// MCP GPIO Server for Raspberry Pi
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// List of pins allowed for Output (Write/Mode change)
    #[arg(long, value_delimiter = ',')]
    allow_out: Vec<u8>,

    /// List of pins allowed for Input (Read only)
    #[arg(long, value_delimiter = ',')]
    allow_in: Vec<u8>,
    
    /// Minimum time between write operations (ms) to prevent thrashing
    #[arg(long, default_value_t = 100)]
    rate_limit_ms: u64,
}

// Minimal JSON-RPC types (omitted for brevity, same as before)
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

#[derive(Debug, Serialize)]
struct Tool {
    name: String,
    description: String,
    #[serde(rename = "inputSchema")]
    input_schema: Value,
}

#[allow(dead_code)]
struct GpioState {
    #[cfg(target_arch = "aarch64")]
    gpio: Gpio,
    allowed_out: HashSet<u8>,
    allowed_in: HashSet<u8>,
    last_write: HashMap<u8, Instant>,
    rate_limit: Duration,
}

impl GpioState {
    pub fn new(allow_out: Vec<u8>, allow_in: Vec<u8>, rate_limit_ms: u64) -> Result<Self> {
        #[cfg(target_arch = "aarch64")]
        let gpio = Gpio::new().context("Failed to initialize GPIO")?;

        Ok(Self {
            #[cfg(target_arch = "aarch64")]
            gpio,
            allowed_out: allow_out.into_iter().collect(),
            allowed_in: allow_in.into_iter().collect(),
            last_write: HashMap::new(),
            rate_limit: Duration::from_millis(rate_limit_ms),
        })
    }

    #[cfg(target_arch = "aarch64")]
    fn check_write_allowed(&mut self, pin: u8) -> Result<(), String> {
        if !self.allowed_out.contains(&pin) {
            return Err(format!("Write access to GPIO pin {} is denied", pin));
        }
        
        let now = Instant::now();
        if let Some(last) = self.last_write.get(&pin) {
            if now.duration_since(*last) < self.rate_limit {
                 return Err("Rate limit exceeded".into());
            }
        }
        self.last_write.insert(pin, now);
        Ok(())
    }

    #[cfg(target_arch = "aarch64")]
    fn check_read_allowed(&self, pin: u8) -> Result<(), String> {
         if self.allowed_in.contains(&pin) || self.allowed_out.contains(&pin) {
            Ok(())
        } else {
             Err(format!("Read access to GPIO pin {} is denied", pin))
        }
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();

    let args = Args::parse();
    info!("Starting MCP GPIO Server.");
    info!("Allowed Out: {:?}", args.allow_out);
    info!("Allowed In: {:?}", args.allow_in);

    // Initialize GPIO
    let state = match GpioState::new(args.allow_out, args.allow_in, args.rate_limit_ms) {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to initialize GPIO: {}. Running in mock mode.", e);
             GpioState { 
                #[cfg(target_arch = "aarch64")]
                gpio: unsafe { std::mem::zeroed() },
                allowed_out: HashSet::new(),
                allowed_in: HashSet::new(),
                last_write: HashMap::new(),
                rate_limit: Duration::from_millis(100),
            }
        }
    };
    
    let state = Arc::new(Mutex::new(state));
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() { continue; }

        let req: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                error!("Invalid JSON-RPC request: {}", e);
                continue;
            }
        };

        if let Some(id) = req.id {
            let resp = handle_request(&req.method, req.params, &state, id.clone());
            let resp_json = serde_json::to_string(&resp)?;
            writeln!(stdout, "{}", resp_json)?;
            stdout.flush()?;
        } else {
             handle_notification(&req.method, req.params);
        }
    }

    Ok(())
}

fn handle_notification(method: &str, _params: Option<Value>) {
    if method == "notifications/initialized" {
        info!("Client initialized");
    }
}

fn handle_request(method: &str, params: Option<Value>, state: &Arc<Mutex<GpioState>>, id: Value) -> JsonRpcResponse {
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "ironclaw-mcp-gpio", "version": "0.1.1" }
        })),
        "tools/list" => Ok(json!({
            "tools": [
                Tool {
                    name: "gpio_write".to_string(),
                    description: "Write logic level to GPIO pin (Output)".to_string(),
                    input_schema: json!({
                         "type": "object",
                        "properties": {
                            "pin": { "type": "integer", "description": "GPIO BCM pin number" },
                            "level": { "type": "string", "enum": ["high", "low"] }
                        },
                        "required": ["pin", "level"]
                    }),
                },
                Tool {
                    name: "gpio_read".to_string(),
                    description: "Read logic level from GPIO pin".to_string(),
                    input_schema: json!({
                         "type": "object",
                        "properties": {
                            "pin": { "type": "integer", "description": "GPIO BCM pin number" }
                        },
                        "required": ["pin"]
                    }),
                }
            ]
        })),
        "tools/call" => {
            if let Some(p) = params {
                 let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
                 let args = p.get("arguments").cloned().unwrap_or(json!({}));
                 call_tool(name, args, state)
            } else {
                Err(JsonRpcError { code: -32602, message: "Missing params".into(), data: None })
            }
        },
        _ => Err(JsonRpcError { code: -32601, message: "Method not found".into(), data: None }),
    };

    match result {
        Ok(res) => JsonRpcResponse { jsonrpc: "2.0".to_string(), id, result: Some(res), error: None },
        Err(err) => JsonRpcResponse { jsonrpc: "2.0".to_string(), id, result: None, error: Some(err) },
    }
}

fn call_tool(name: &str, args: Value, state: &Arc<Mutex<GpioState>>) -> Result<Value, JsonRpcError> {
    #[cfg(not(target_arch = "aarch64"))]
    {
        return Err(JsonRpcError { code: -32000, message: "GPIO tools only available on aarch64".into(), data: None });
    }

    #[cfg(target_arch = "aarch64")]
    {
        let mut state = state.lock().map_err(|_| JsonRpcError { code: -32000, message: "Mutex poisoned".into(), data: None })?;
        
        let pin_num = args.get("pin").and_then(|v| v.as_u64()).ok_or(JsonRpcError { code: -32602, message: "Missing pin".into(), data: None })? as u8;

        let pin = match state.gpio.get(pin_num) {
            Ok(p) => p,
            Err(e) => return Err(JsonRpcError { code: -32000, message: format!("Failed to get pin: {}", e), data: None }),
        };

        match name {
            "gpio_write" => {
                if let Err(e) = state.check_write_allowed(pin_num) {
                    return Err(JsonRpcError { code: -32000, message: e, data: None });
                }
                
                let level = args.get("level").and_then(|v| v.as_str()).unwrap_or("low");
                let mut output_pin = pin.into_output();
                match level {
                    "high" => output_pin.set_high(),
                    "low" => output_pin.set_low(),
                    _ => return Err(JsonRpcError { code: -32602, message: "Invalid level".into(), data: None }),
                }
                Ok(json!({ "content": [{ "type": "text", "text": format!("Set pin {} to {}", pin_num, level) }] }))
            },
            "gpio_read" => {
                if let Err(e) = state.check_read_allowed(pin_num) {
                    return Err(JsonRpcError { code: -32000, message: e, data: None });
                }
                let level = if pin.read() == Level::High { "high" } else { "low" };
                Ok(json!({ "content": [{ "type": "text", "text": level }] }))
            },
             _ => Err(JsonRpcError { code: -32601, message: "Tool not found".into(), data: None }),
        }
    }
}
