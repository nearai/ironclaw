//! Baidu Maps WASM Tool for IronClaw.
//!
//! Provides geocoding, reverse geocoding, POI search, and route planning
//! via the Baidu Maps API.
//!
//! # Authentication
//!
//! Store your Baidu Maps AK:
//! `ironclaw secret set baidu_map_ak <key>`
//!
//! Get an AK at: https://lbsyun.baidu.com/

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::Deserialize;

const BASE_URL: &str = "https://api.map.baidu.com";
const MAX_RETRIES: u32 = 3;

struct BaiduMapTool;

impl exports::near::agent::tool::Guest for BaiduMapTool {
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
        "百度地图 API — 地理编码、逆地理编码、POI 搜索、路径规划。\
         Authentication is handled via the 'baidu_map_ak' secret injected by the host."
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
struct Params {
    action: String,
    address: Option<String>,
    lat: Option<f64>,
    lng: Option<f64>,
    query: Option<String>,
    region: Option<String>,
    dest_lat: Option<f64>,
    dest_lng: Option<f64>,
}

// --- Geocode response ---
#[derive(Debug, Deserialize)]
struct GeocodeResponse {
    status: i32,
    result: Option<GeocodeResult>,
}

#[derive(Debug, Deserialize)]
struct GeocodeResult {
    location: Option<Location>,
    level: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Location {
    lng: f64,
    lat: f64,
}

// --- Reverse geocode response ---
#[derive(Debug, Deserialize)]
struct ReverseGeocodeResponse {
    status: i32,
    result: Option<ReverseGeocodeResult>,
}

#[derive(Debug, Deserialize)]
struct ReverseGeocodeResult {
    formatted_address: Option<String>,
    #[serde(rename = "addressComponent")]
    address_component: Option<serde_json::Value>,
}

// --- Place search response ---
#[derive(Debug, Deserialize)]
struct PlaceSearchResponse {
    status: i32,
    results: Option<Vec<PlaceResult>>,
}

#[derive(Debug, Deserialize)]
struct PlaceResult {
    name: Option<String>,
    address: Option<String>,
    location: Option<Location>,
    telephone: Option<String>,
    uid: Option<String>,
}

// --- Route plan response ---
#[derive(Debug, Deserialize)]
struct RoutePlanResponse {
    status: i32,
    result: Option<RoutePlanResult>,
}

#[derive(Debug, Deserialize)]
struct RoutePlanResult {
    routes: Option<Vec<Route>>,
}

#[derive(Debug, Deserialize)]
struct Route {
    distance: Option<f64>,
    duration: Option<f64>,
    #[serde(default)]
    steps: Vec<RouteStep>,
}

#[derive(Debug, Deserialize)]
struct RouteStep {
    instruction: Option<String>,
    distance: Option<f64>,
    duration: Option<f64>,
}

fn execute_inner(params: &str) -> Result<String, String> {
    let params: Params =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    if !near::agent::host::secret_exists("baidu_map_ak") {
        return Err(
            "Baidu Maps AK not found in secret store. Set it with: \
             ironclaw secret set baidu_map_ak <key>. \
             Get an AK at: https://lbsyun.baidu.com/"
                .into(),
        );
    }

    match params.action.as_str() {
        "geocode" => action_geocode(&params),
        "reverse_geocode" => action_reverse_geocode(&params),
        "place_search" => action_place_search(&params),
        "route_plan" => action_route_plan(&params),
        other => Err(format!(
            "Unknown action '{other}'. Valid actions: geocode, reverse_geocode, place_search, route_plan"
        )),
    }
}

fn action_geocode(params: &Params) -> Result<String, String> {
    let address = params
        .address
        .as_deref()
        .ok_or("'address' is required for geocode action")?;
    if address.is_empty() {
        return Err("'address' must not be empty".into());
    }

    let encoded = url_encode(address);
    let url = format!("{BASE_URL}/geocoding/v3/?address={encoded}&output=json");

    let body = do_get(&url)?;
    let resp: GeocodeResponse =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse geocode response: {e}"))?;

    if resp.status != 0 {
        return Err(format!("Baidu API error (status {})", resp.status));
    }

    let result = resp.result.ok_or("No geocode result returned")?;
    let location = result.location.ok_or("No location in geocode result")?;

    let output = serde_json::json!({
        "action": "geocode",
        "address": address,
        "lng": location.lng,
        "lat": location.lat,
        "level": result.level,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn action_reverse_geocode(params: &Params) -> Result<String, String> {
    let lat = params.lat.ok_or("'lat' is required for reverse_geocode action")?;
    let lng = params.lng.ok_or("'lng' is required for reverse_geocode action")?;

    let url = format!("{BASE_URL}/reverse_geocoding/v3/?location={lat},{lng}&output=json");

    let body = do_get(&url)?;
    let resp: ReverseGeocodeResponse = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse reverse geocode response: {e}"))?;

    if resp.status != 0 {
        return Err(format!("Baidu API error (status {})", resp.status));
    }

    let result = resp.result.ok_or("No reverse geocode result returned")?;

    let output = serde_json::json!({
        "action": "reverse_geocode",
        "lat": lat,
        "lng": lng,
        "formatted_address": result.formatted_address,
        "address_component": result.address_component,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn action_place_search(params: &Params) -> Result<String, String> {
    let query = params
        .query
        .as_deref()
        .ok_or("'query' is required for place_search action")?;
    if query.is_empty() {
        return Err("'query' must not be empty".into());
    }

    let region = params.region.as_deref().unwrap_or("全国");
    let encoded_query = url_encode(query);
    let encoded_region = url_encode(region);
    let url = format!(
        "{BASE_URL}/place/v2/search?query={encoded_query}&region={encoded_region}&output=json&page_size=20"
    );

    let body = do_get(&url)?;
    let resp: PlaceSearchResponse = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse place search response: {e}"))?;

    if resp.status != 0 {
        return Err(format!("Baidu API error (status {})", resp.status));
    }

    let results = resp.results.unwrap_or_default();
    let formatted: Vec<serde_json::Value> = results
        .into_iter()
        .filter_map(|r| {
            let name = r.name?;
            let mut entry = serde_json::json!({"name": name});
            if let Some(addr) = r.address {
                entry["address"] = serde_json::json!(addr);
            }
            if let Some(loc) = r.location {
                entry["lng"] = serde_json::json!(loc.lng);
                entry["lat"] = serde_json::json!(loc.lat);
            }
            if let Some(tel) = r.telephone {
                entry["telephone"] = serde_json::json!(tel);
            }
            if let Some(uid) = r.uid {
                entry["uid"] = serde_json::json!(uid);
            }
            Some(entry)
        })
        .collect();

    let output = serde_json::json!({
        "action": "place_search",
        "query": query,
        "region": region,
        "result_count": formatted.len(),
        "results": formatted,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn action_route_plan(params: &Params) -> Result<String, String> {
    let lat = params.lat.ok_or("'lat' (origin) is required for route_plan action")?;
    let lng = params.lng.ok_or("'lng' (origin) is required for route_plan action")?;
    let dest_lat = params
        .dest_lat
        .ok_or("'dest_lat' is required for route_plan action")?;
    let dest_lng = params
        .dest_lng
        .ok_or("'dest_lng' is required for route_plan action")?;

    let url = format!(
        "{BASE_URL}/direction/v2/driving?origin={lat},{lng}&destination={dest_lat},{dest_lng}&output=json"
    );

    let body = do_get(&url)?;
    let resp: RoutePlanResponse = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse route plan response: {e}"))?;

    if resp.status != 0 {
        return Err(format!("Baidu API error (status {})", resp.status));
    }

    let result = resp.result.ok_or("No route plan result returned")?;
    let routes = result.routes.unwrap_or_default();

    let formatted: Vec<serde_json::Value> = routes
        .into_iter()
        .map(|r| {
            let steps: Vec<serde_json::Value> = r
                .steps
                .into_iter()
                .filter_map(|s| {
                    let instruction = s.instruction?;
                    let mut step = serde_json::json!({"instruction": instruction});
                    if let Some(d) = s.distance {
                        step["distance_m"] = serde_json::json!(d);
                    }
                    if let Some(d) = s.duration {
                        step["duration_s"] = serde_json::json!(d);
                    }
                    Some(step)
                })
                .collect();

            let mut route = serde_json::json!({"steps": steps});
            if let Some(d) = r.distance {
                route["total_distance_m"] = serde_json::json!(d);
            }
            if let Some(d) = r.duration {
                route["total_duration_s"] = serde_json::json!(d);
            }
            route
        })
        .collect();

    let output = serde_json::json!({
        "action": "route_plan",
        "origin": {"lat": lat, "lng": lng},
        "destination": {"lat": dest_lat, "lng": dest_lng},
        "route_count": formatted.len(),
        "routes": formatted,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn do_get(url: &str) -> Result<String, String> {
    let headers = serde_json::json!({
        "User-Agent": "IronClaw-BaiduMap-Tool/0.1",
        "Accept": "application/json"
    });

    let mut attempt = 0;
    loop {
        attempt += 1;

        let resp = near::agent::host::http_request("GET", url, &headers.to_string(), None, None)
            .map_err(|e| format!("HTTP request failed: {e}"))?;

        if resp.status >= 200 && resp.status < 300 {
            return String::from_utf8(resp.body)
                .map_err(|e| format!("Invalid UTF-8 response: {e}"));
        }

        if attempt < MAX_RETRIES && (resp.status == 429 || resp.status >= 500) {
            near::agent::host::log(
                near::agent::host::LogLevel::Warn,
                &format!(
                    "Baidu Maps API error {} (attempt {}/{}). Retrying...",
                    resp.status, attempt, MAX_RETRIES
                ),
            );
            continue;
        }

        let body = String::from_utf8_lossy(&resp.body);
        return Err(format!("Baidu Maps API error (HTTP {}): {}", resp.status, body));
    }
}

fn url_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            _ => {
                result.push('%');
                result.push_str(&format!("{byte:02X}"));
            }
        }
    }
    result
}

const SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "action": {
            "type": "string",
            "description": "The action to perform",
            "enum": ["geocode", "reverse_geocode", "place_search", "route_plan"]
        },
        "address": {
            "type": "string",
            "description": "Address to geocode (required for 'geocode' action)"
        },
        "lat": {
            "type": "number",
            "description": "Latitude (required for 'reverse_geocode' and 'route_plan' as origin)"
        },
        "lng": {
            "type": "number",
            "description": "Longitude (required for 'reverse_geocode' and 'route_plan' as origin)"
        },
        "query": {
            "type": "string",
            "description": "Search query for POI (required for 'place_search')"
        },
        "region": {
            "type": "string",
            "description": "Region to search in (for 'place_search', default '全国')",
            "default": "全国"
        },
        "dest_lat": {
            "type": "number",
            "description": "Destination latitude (required for 'route_plan')"
        },
        "dest_lng": {
            "type": "number",
            "description": "Destination longitude (required for 'route_plan')"
        }
    },
    "required": ["action"],
    "additionalProperties": false
}"#;

export!(BaiduMapTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_encode() {
        assert_eq!(url_encode("hello"), "hello");
        assert_eq!(url_encode("hello world"), "hello%20world");
        assert_eq!(url_encode("北京"), "%E5%8C%97%E4%BA%AC");
    }

    #[test]
    fn test_parse_geocode_response() {
        let json = r#"{
            "status": 0,
            "result": {
                "location": {"lng": 116.307484, "lat": 40.056878},
                "level": "门址"
            }
        }"#;
        let resp: GeocodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, 0);
        let result = resp.result.unwrap();
        let loc = result.location.unwrap();
        assert!((loc.lng - 116.307484).abs() < 0.0001);
        assert!((loc.lat - 40.056878).abs() < 0.0001);
        assert_eq!(result.level.as_deref(), Some("门址"));
    }

    #[test]
    fn test_parse_reverse_geocode_response() {
        let json = r#"{
            "status": 0,
            "result": {
                "formatted_address": "北京市海淀区中关村大街1号",
                "addressComponent": {"city": "北京市"}
            }
        }"#;
        let resp: ReverseGeocodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, 0);
        let result = resp.result.unwrap();
        assert_eq!(
            result.formatted_address.as_deref(),
            Some("北京市海淀区中关村大街1号")
        );
    }

    #[test]
    fn test_parse_place_search_response() {
        let json = r#"{
            "status": 0,
            "results": [
                {
                    "name": "星巴克",
                    "address": "中关村大街1号",
                    "location": {"lng": 116.307, "lat": 40.056},
                    "telephone": "010-12345678",
                    "uid": "abc123"
                }
            ]
        }"#;
        let resp: PlaceSearchResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, 0);
        let results = resp.results.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name.as_deref(), Some("星巴克"));
    }

    #[test]
    fn test_parse_place_search_empty() {
        let json = r#"{"status": 0, "results": []}"#;
        let resp: PlaceSearchResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, 0);
        assert!(resp.results.unwrap().is_empty());
    }

    #[test]
    fn test_parse_route_plan_response() {
        let json = r#"{
            "status": 0,
            "result": {
                "routes": [
                    {
                        "distance": 12500.0,
                        "duration": 1800.0,
                        "steps": [
                            {
                                "instruction": "向东行驶500米",
                                "distance": 500.0,
                                "duration": 60.0
                            }
                        ]
                    }
                ]
            }
        }"#;
        let resp: RoutePlanResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, 0);
        let result = resp.result.unwrap();
        let routes = result.routes.unwrap();
        assert_eq!(routes.len(), 1);
        assert!((routes[0].distance.unwrap() - 12500.0).abs() < 0.1);
        assert_eq!(routes[0].steps.len(), 1);
    }

    #[test]
    fn test_parse_route_plan_empty() {
        let json = r#"{"status": 0, "result": {"routes": []}}"#;
        let resp: RoutePlanResponse = serde_json::from_str(json).unwrap();
        assert!(resp.result.unwrap().routes.unwrap().is_empty());
    }
}
