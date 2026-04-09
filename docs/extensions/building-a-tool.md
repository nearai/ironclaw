---
title: Build a Tool from Scratch
description: "Build a weather tool from scratch with Rust"
---

In this tutorial you will build **weather-tool** from scratch — a WASM tool that fetches current conditions, a 5-day forecast, and air quality data using the free [Open-Meteo](https://open-meteo.com) API (no API key required).

By the end you will have a working tool your agent can call like this:

> "What's the weather in Tokyo right now?"

The complete source code for this tool is available on GitHub:

<Card title="weather-tool source" icon="github" href="https://github.com/matiasbenary/ironclaw/tree/tools/weather/tools-src/weather">
  Browse the full implementation — `lib.rs`, `Cargo.toml`, and `weather-tool.capabilities.json`.
</Card>

---

## Prerequisites

If you don't have Rust yet, install it from [rustup.rs](https://rustup.rs):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Then add the WASM target:

```bash
rustup target add wasm32-wasip2
```

---

## 1. Create the project

```bash
cargo new --lib weather-tool
cd weather-tool
```

Replace the generated `Cargo.toml` with:

```toml Cargo.toml
[package]
name = "weather-tool"
version = "0.1.0"
edition = "2021"
description = "Weather information tool for IronClaw (WASM component)"

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "=0.36"
serde = { version = "1", features = ["derive"] }
serde_json = "1"

[profile.release]
opt-level = "s"
lto = true
strip = true
codegen-units = 1

[workspace]
```

<Note>
`crate-type = ["cdylib"]` tells Cargo to produce a dynamic library — the format WASM components require. `[workspace]` stops Cargo from merging this crate into a parent workspace.
</Note>

---

## 2. Wire up the WIT interface

Every IronClaw tool is a WASM component that implements a WIT interface. The host provides HTTP, logging, and workspace capabilities; your tool exports `execute`, `schema`, and `description`.

Replace `src/lib.rs` with the following skeleton:

```rust src/lib.rs
wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit", // path relative to your Cargo.toml
});

use serde::{Deserialize, Serialize};

struct WeatherTool;

impl exports::near::agent::tool::Guest for WeatherTool {
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
        SCHEMA.to_string()
    }

    fn description() -> String {
        "Get weather information using Open-Meteo (no API key required). \
         Supports three actions: 'get_current' returns current weather conditions \
         for a city; 'get_forecast' returns a 5-day daily forecast; \
         'get_air_quality' returns air pollution data for given coordinates."
            .to_string()
    }
}

export!(WeatherTool);
```

`execute_inner` is where the real logic lives — you will fill it in next.

<Note>
The `wit/tool.wit` file ships with IronClaw. If you are building inside the IronClaw repo (e.g. under `tools-src/my-tool/`), the path `../../wit/tool.wit` is correct. If you are building in a standalone directory, copy `wit/tool.wit` from the repo root and adjust the path accordingly.
</Note>

<Note>
If your tool uses private credentials (API keys, OAuth tokens), you still keep the same WIT interface. Secret handling is declared in `*.capabilities.json` and injected by the host at runtime. Your WASM tool should not ask the model for secrets in `params`.
</Note>

---

## 3. Define the Execute Logic

The tool will receive parameters provided by the LLM in JSON format, then execute the right logic based on those parameters and return a result also in JSON format.

```rust src/lib.rs
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum Action {
    GetCurrent(WeatherParams),
    GetForecast(WeatherParams),
    GetAirQuality(AirQualityParams),
}

#[derive(Debug, Deserialize)]
struct WeatherParams {
    city: String,
    #[serde(default)]
    country_code: Option<String>,
    #[serde(default)]
    units: Option<String>, // "metric" (default) or "imperial"
}

#[derive(Debug, Deserialize)]
struct AirQualityParams {
    lat: f64,
    lon: f64,
}

fn execute_inner(params: &str) -> Result<String, String> {
    let action: Action =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    match action {
        Action::GetCurrent(p)    => get_current(p),
        Action::GetForecast(p)   => get_forecast(p),
        Action::GetAirQuality(p) => get_air_quality(p),
    }
}
```

<Note>

Remember to match the action names and parameter structure in the JSON schema you will define later. The LLM relies on that schema to know what JSON to send, so if your Rust code expects `country_code` but the schema calls it `country`, the LLM won't know to include it and you'll get errors at runtime.

</Note>

---

## 4. Implement the Actions

We will now implement the three actions: `get_current`, `get_forecast`, and `get_air_quality`. Each action will call the appropriate Open-Meteo API endpoint, parse the response, and return a JSON string with the relevant information.

<Accordion title="Handling authenticated APIs" icon="key">

If your API needs a secret (for example a bearer token), you do not inject it in these Rust functions manually. 

Instead you will declare them in the [capabilities file](#9-add-secrets-and-auth-for-tools-that-need-credentials) and let the host inject them at runtime.

Your Rust code just calls `api_get(...)` with the right URL and headers, and the host adds credentials automatically for allowlisted hosts.

You can still check for the presence of secrets if you want to return a custom error message when credentials are missing:

```rust
if !near::agent::host::secret_exists("example_api_token") {
        return Err("Missing secret: example_api_token. Run: ironclaw tool auth <tool-name>".into());
}
```

</Accordion>


### Geocoding helper

Open-Meteo needs coordinates, not city names. Add a helper that calls the free geocoding API:

```rust src/lib.rs
#[derive(Debug, Deserialize)]
struct GeoResult {
    latitude: f64,
    longitude: f64,
    name: String,
    country: String,
}

fn geocode(city: &str, country_code: Option<&str>) -> Result<GeoResult, String> {
    let mut url = format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=1&language=en&format=json",
        url_encode(city)
    );
    if let Some(cc) = country_code {
        if !cc.is_empty() {
            url.push_str(&format!("&countryCode={}", url_encode(cc)));
        }
    }

    near::agent::host::log(
        near::agent::host::LogLevel::Info,
        &format!("Geocoding: {city}"),
    );

    let resp = api_get(&url)?;
    let data: serde_json::Value =
        serde_json::from_str(&resp).map_err(|e| format!("Failed to parse geocoding: {e}"))?;

    let results = data["results"]
        .as_array()
        .ok_or_else(|| format!("City not found: {city}"))?;

    if results.is_empty() {
        return Err(format!("City not found: {city}"));
    }

    let r = &results[0];
    Ok(GeoResult {
        latitude:  r["latitude"].as_f64().unwrap_or(0.0),
        longitude: r["longitude"].as_f64().unwrap_or(0.0),
        name:      r["name"].as_str().unwrap_or(city).to_string(),
        country:   r["country"].as_str().unwrap_or("").to_string(),
    })
}
```

`near::agent::host::log` emits a structured log line visible in `ironclaw` output. The host collects all log entries and flushes them after the call completes.

### API helper

```rust src/lib.rs
fn api_get(url: &str) -> Result<String, String> {
    let headers = serde_json::json!({
        "Accept": "application/json",
        "User-Agent": "IronClaw-Weather-Tool/0.1"
    }).to_string();

    let resp = near::agent::host::http_request("GET", url, &headers, None, None)
        .map_err(|e| format!("HTTP request failed: {e}"))?;

    if resp.status < 200 || resp.status >= 300 {
        return Err(format!("API error (HTTP {}): {}", resp.status,
            String::from_utf8_lossy(&resp.body)));
    }

    String::from_utf8(resp.body).map_err(|e| format!("Invalid UTF-8 response: {e}"))
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push_str("%20"),
            _ => {
                out.push('%');
                out.push(char::from(b"0123456789ABCDEF"[(b >> 4) as usize]));
                out.push(char::from(b"0123456789ABCDEF"[(b & 0xf) as usize]));
            }
        }
    }
    out
}

fn wmo_description(code: u32) -> String {
    match code {
        0  => "Clear sky",
        1  => "Mainly clear",
        2  => "Partly cloudy",
        3  => "Overcast",
        45 => "Fog",
        51 => "Light drizzle",
        61 => "Slight rain",
        63 => "Moderate rain",
        65 => "Heavy rain",
        71 => "Slight snow",
        73 => "Moderate snow",
        75 => "Heavy snow",
        80 => "Slight rain showers",
        95 => "Thunderstorm",
        _  => "Unknown",
    }.to_string()
}

fn european_aqi_label(aqi: u32) -> String {
    match aqi {
        0..=20  => "Good",
        21..=40 => "Fair",
        41..=60 => "Moderate",
        61..=80 => "Poor",
        81..=100 => "Very Poor",
        _        => "Extremely Poor",
    }.to_string()
}
```

### Get current weather

```rust 
fn get_current(params: WeatherParams) -> Result<String, String> {
    if params.city.is_empty() {
        return Err("'city' must not be empty".into());
    }

    let geo = geocode(&params.city, params.country_code.as_deref())?;
    let units     = params.units.as_deref().unwrap_or("metric");
    let temp_unit = if units == "imperial" { "fahrenheit" } else { "celsius" };
    let wind_unit = if units == "imperial" { "mph" } else { "ms" };

    let url = format!(
        "https://api.open-meteo.com/v1/forecast\
         ?latitude={}&longitude={}\
         &current=temperature_2m,apparent_temperature,relative_humidity_2m,\
         weather_code,wind_speed_10m\
         &temperature_unit={}&wind_speed_unit={}",
        geo.latitude, geo.longitude, temp_unit, wind_unit
    );

    let resp = api_get(&url)?;
    let data: serde_json::Value =
        serde_json::from_str(&resp).map_err(|e| format!("Failed to parse response: {e}"))?;

    let current = &data["current"];
    let output = CurrentWeatherOutput {
        city:        geo.name,
        country:     geo.country,
        temperature: current["temperature_2m"].as_f64().unwrap_or(0.0),
        feels_like:  current["apparent_temperature"].as_f64().unwrap_or(0.0),
        humidity:    current["relative_humidity_2m"].as_u64().unwrap_or(0) as u32,
        description: wmo_description(current["weather_code"].as_u64().unwrap_or(0) as u32),
        wind_speed:  current["wind_speed_10m"].as_f64().unwrap_or(0.0),
        units:       units.to_string(),
    };

    serde_json::to_string(&output).map_err(|e| format!("Serialization error: {e}"))
}
```

### Get forecast

```rust src/lib.rs
fn get_forecast(params: WeatherParams) -> Result<String, String> {
    if params.city.is_empty() {
        return Err("'city' must not be empty".into());
    }

    let geo       = geocode(&params.city, params.country_code.as_deref())?;
    let units     = params.units.as_deref().unwrap_or("metric");
    let temp_unit = if units == "imperial" { "fahrenheit" } else { "celsius" };
    let wind_unit = if units == "imperial" { "mph" } else { "ms" };

    let url = format!(
        "https://api.open-meteo.com/v1/forecast\
         ?latitude={}&longitude={}\
         &daily=temperature_2m_max,temperature_2m_min,weather_code,\
         precipitation_probability_max\
         &temperature_unit={}&wind_speed_unit={}&forecast_days=5",
        geo.latitude, geo.longitude, temp_unit, wind_unit
    );

    let resp  = api_get(&url)?;
    let data: serde_json::Value =
        serde_json::from_str(&resp).map_err(|e| format!("Failed to parse response: {e}"))?;

    let daily    = &data["daily"];
    let times    = daily["time"].as_array().cloned().unwrap_or_default();
    let temp_max = daily["temperature_2m_max"].as_array().cloned().unwrap_or_default();
    let temp_min = daily["temperature_2m_min"].as_array().cloned().unwrap_or_default();
    let codes    = daily["weather_code"].as_array().cloned().unwrap_or_default();
    let precip   = daily["precipitation_probability_max"].as_array().cloned().unwrap_or_default();

    let entries = times.iter().enumerate().map(|(i, t)| ForecastEntry {
        date:        t.as_str().unwrap_or("").to_string(),
        temp_max:    temp_max.get(i).and_then(|v| v.as_f64()).unwrap_or(0.0),
        temp_min:    temp_min.get(i).and_then(|v| v.as_f64()).unwrap_or(0.0),
        description: wmo_description(codes.get(i).and_then(|v| v.as_u64()).unwrap_or(0) as u32),
        precipitation_probability_max: precip.get(i).and_then(|v| v.as_u64()).unwrap_or(0) as u32,
    }).collect();

    let output = ForecastOutput { city: geo.name, country: geo.country, units: units.to_string(), entries };
    serde_json::to_string(&output).map_err(|e| format!("Serialization error: {e}"))
}
```

### Get air quality

```rust src/lib.rs
fn get_air_quality(params: AirQualityParams) -> Result<String, String> {
    if params.lat < -90.0 || params.lat > 90.0 {
        return Err(format!("'lat' must be -90..90, got {}", params.lat));
    }
    if params.lon < -180.0 || params.lon > 180.0 {
        return Err(format!("'lon' must be -180..180, got {}", params.lon));
    }

    let url = format!(
        "https://air-quality-api.open-meteo.com/v1/air-quality\
         ?latitude={}&longitude={}\
         &current=pm10,pm2_5,european_aqi",
        params.lat, params.lon
    );

    let resp = api_get(&url)?;
    let data: serde_json::Value =
        serde_json::from_str(&resp).map_err(|e| format!("Failed to parse response: {e}"))?;

    let current = &data["current"];
    let aqi     = current["european_aqi"].as_u64().unwrap_or(0) as u32;

    let output = AirQualityOutput {
        lat:           params.lat,
        lon:           params.lon,
        european_aqi:  aqi,
        aqi_label:     european_aqi_label(aqi),
        pm2_5:         current["pm2_5"].as_f64().unwrap_or(0.0),
        pm10:          current["pm10"].as_f64().unwrap_or(0.0),
    };

    serde_json::to_string(&output).map_err(|e| format!("Serialization error: {e}"))
}
```

---

## 5. Define the JSON schema

The `SCHEMA` constant tells the LLM exactly what JSON to send. Use `oneOf` because the three actions have different required fields:

```rust src/lib.rs
const SCHEMA: &str = r#"{
    "oneOf": [
        {
            "type": "object",
            "description": "Get current weather conditions for a city",
            "properties": {
                "action":       { "type": "string", "const": "get_current" },
                "city":         { "type": "string", "description": "City name, e.g. 'Tokyo'" },
                "country_code": { "type": "string", "description": "ISO 3166-1 alpha-2 code, e.g. 'JP'" },
                "units":        { "type": "string", "enum": ["metric", "imperial"] }
            },
            "required": ["action", "city"],
            "additionalProperties": false
        },
        {
            "type": "object",
            "description": "Get a 5-day daily weather forecast for a city",
            "properties": {
                "action":       { "type": "string", "const": "get_forecast" },
                "city":         { "type": "string" },
                "country_code": { "type": "string" },
                "units":        { "type": "string", "enum": ["metric", "imperial"] }
            },
            "required": ["action", "city"],
            "additionalProperties": false
        },
        {
            "type": "object",
            "description": "Get air quality data for a location by coordinates",
            "properties": {
                "action": { "type": "string", "const": "get_air_quality" },
                "lat":    { "type": "number", "description": "Latitude (-90 to 90)" },
                "lon":    { "type": "number", "description": "Longitude (-180 to 180)" }
            },
            "required": ["action", "lat", "lon"],
            "additionalProperties": false
        }
    ]
}"#;
```

---

## 6. Declare capabilities

Create `weather-tool.capabilities.json` next to `Cargo.toml`. This file is the sandbox allowlist — any host not listed here is blocked at runtime:

```json weather-tool.capabilities.json
{
  "version": "0.1.0",
  "wit_version": "0.3.0",
  "http": {
    "allowlist": [
      {
        "host": "geocoding-api.open-meteo.com",
        "path_prefix": "/v1/",
        "methods": ["GET"]
      },
      {
        "host": "api.open-meteo.com",
        "path_prefix": "/v1/",
        "methods": ["GET"]
      },
      {
        "host": "air-quality-api.open-meteo.com",
        "path_prefix": "/v1/",
        "methods": ["GET"]
      }
    ],
    "rate_limit": {
      "requests_per_minute": 60,
      "requests_per_hour": 500
    },
    "timeout_secs": 15
  }
}
```

The weather tool needs three hosts because `get_current` and `get_forecast` make two requests each: one to geocode the city name and one to fetch the weather data.

---

## 7. Add secrets and auth (for tools that need credentials)

This weather tool uses Open-Meteo, so it does not need a secret. If your tool calls an API that needs a token, declare that in the capabilities file so IronClaw can inject it at request time.

Example capability sections (pattern used in `tools-src/*` on the IronClaw repo):

```json weather-tool.capabilities.json
{
    "http": {
        "allowlist": [
            {
                "host": "api.example.com",
                "path_prefix": "/v1/",
                "methods": ["GET", "POST"]
            }
        ],
        "credentials": {
            "example_api_token": {
                "secret_name": "example_api_token",
                "location": { "type": "bearer" },
                "host_patterns": ["api.example.com"]
            }
        }
    },
    "secrets": {
        "allowed_names": ["example_api_token"]
    },
    "auth": {
        "secret_name": "example_api_token",
        "display_name": "Example API",
        "instructions": "Create an API token in your provider dashboard",
        "setup_url": "https://example.com/settings/api",
        "token_hint": "Starts with 'ex_'",
        "env_var": "EXAMPLE_API_TOKEN"
    }
}
```

How this works:

- `http.credentials` maps a stored secret to where it should be injected (`bearer`, custom header, query param, or URL placeholder).
- `secrets.allowed_names` lets the tool check secret presence with `near::agent::host::secret_exists(...)`.
- `auth` tells IronClaw how to collect credentials.

After installing the tool, run auth once:

```bash
ironclaw tool auth <tool-name>
```

Auth flow priority is:

1. Use `auth.env_var` if it is set in your environment.
2. Use OAuth if `auth.oauth` is configured.
3. Fall back to manual token entry using `instructions` and `setup_url`.

If your capabilities include `setup.required_secrets` (for example OAuth client id/client secret fields), run setup as well:

```bash
ironclaw tool setup <tool-name>
```

This keeps credentials outside agent-visible prompts and lets the host inject them only where allowlisted.

---

## 8. Build and install

```bash
cargo build --target wasm32-wasip2 --release
```

```bash
ironclaw tool install ./target/wasm32-wasip2/release/weather_tool.wasm \
  --capabilities ./weather-tool.capabilities.json \
  --name weather-tool
```

Verify it loaded:

```bash
ironclaw tool list
```

If your tool defines secret variables, authenticate now:

```bash
ironclaw tool auth <tool-name>
```

If your tool defines `setup.required_secrets`, run:

```bash
ironclaw tool setup <tool-name>
```

---

## Try it out

Start IronClaw and ask your agent:

- "What's the weather in Buenos Aires?"
- "Give me a 5-day forecast for London, GB in imperial units."
- "What's the air quality at coordinates 35.6762, 139.6503?"

The agent resolves the right action from the schema and calls the tool automatically.
