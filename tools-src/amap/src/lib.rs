//! Amap (Gaode Maps) WASM Tool for IronClaw.
//!
//! Provides geocoding, reverse geocoding, POI search, and route planning
//! via the Amap (高德地图) API.
//!
//! # Authentication
//!
//! Store your Amap Key:
//! `ironclaw secret set amap_key <key>`
//!
//! Get a key at: https://lbs.amap.com/

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::Deserialize;

const BASE_URL: &str = "https://restapi.amap.com";
const MAX_RETRIES: u32 = 3;

struct AmapTool;

impl exports::near::agent::tool::Guest for AmapTool {
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
        "高德地图 API — 地理编码、逆地理编码、POI 搜索、路径规划。\
         Authentication is handled via the 'amap_key' secret injected by the host."
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
    status: String,
    geocodes: Option<Vec<Geocode>>,
}

#[derive(Debug, Deserialize)]
struct Geocode {
    formatted_address: Option<String>,
    location: Option<String>, // "lng,lat"
    level: Option<String>,
}

// --- Reverse geocode response ---
#[derive(Debug, Deserialize)]
struct ReverseGeocodeResponse {
    status: String,
    regeocode: Option<Regeocode>,
}

#[derive(Debug, Deserialize)]
struct Regeocode {
    formatted_address: Option<String>,
    #[serde(rename = "addressComponent")]
    address_component: Option<serde_json::Value>,
}

// --- Place search response (v5) ---
#[derive(Debug, Deserialize)]
struct PlaceSearchResponse {
    status: String,
    pois: Option<Vec<Poi>>,
}

#[derive(Debug, Deserialize)]
struct Poi {
    name: Option<String>,
    address: Option<String>,
    location: Option<String>, // "lng,lat"
    #[serde(rename = "type")]
    poi_type: Option<String>,
    tel: Option<String>,
    id: Option<String>,
    business: Option<serde_json::Value>,
}

// --- Route plan response (v5) ---
#[derive(Debug, Deserialize)]
struct RoutePlanResponse {
    status: String,
    route: Option<RoutePlanRoute>,
}

#[derive(Debug, Deserialize)]
struct RoutePlanRoute {
    paths: Option<Vec<RoutePath>>,
}

#[derive(Debug, Deserialize)]
struct RoutePath {
    distance: Option<String>,
    duration: Option<String>,
    strategy: Option<String>,
    #[serde(default)]
    steps: Vec<RouteStep>,
}

#[derive(Debug, Deserialize)]
struct RouteStep {
    instruction: Option<String>,
    distance: Option<String>,
    duration: Option<String>,
}

fn execute_inner(params: &str) -> Result<String, String> {
    let params: Params =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    if !near::agent::host::secret_exists("amap_key") {
        return Err(
            "Amap Key not found in secret store. Set it with: \
             ironclaw secret set amap_key <key>. \
             Get a key at: https://lbs.amap.com/"
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
    let url = format!("{BASE_URL}/v3/geocode/geo?address={encoded}&output=json");

    let body = do_get(&url)?;
    let resp: GeocodeResponse =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse geocode response: {e}"))?;

    if resp.status != "1" {
        return Err(format!("Amap API error (status {})", resp.status));
    }

    let geocodes = resp.geocodes.unwrap_or_default();
    let first = geocodes.into_iter().next().ok_or("No geocode result returned")?;

    let (lng, lat) = parse_location_str(first.location.as_deref().unwrap_or(""))?;

    let output = serde_json::json!({
        "action": "geocode",
        "address": address,
        "lng": lng,
        "lat": lat,
        "formatted_address": first.formatted_address,
        "level": first.level,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn action_reverse_geocode(params: &Params) -> Result<String, String> {
    let lat = params.lat.ok_or("'lat' is required for reverse_geocode action")?;
    let lng = params.lng.ok_or("'lng' is required for reverse_geocode action")?;

    // Amap uses lng,lat order (not lat,lng)
    let url = format!("{BASE_URL}/v3/geocode/regeo?location={lng},{lat}&output=json");

    let body = do_get(&url)?;
    let resp: ReverseGeocodeResponse = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse reverse geocode response: {e}"))?;

    if resp.status != "1" {
        return Err(format!("Amap API error (status {})", resp.status));
    }

    let regeocode = resp.regeocode.ok_or("No reverse geocode result returned")?;

    let output = serde_json::json!({
        "action": "reverse_geocode",
        "lat": lat,
        "lng": lng,
        "formatted_address": regeocode.formatted_address,
        "address_component": regeocode.address_component,
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

    let region = params.region.as_deref().unwrap_or("");
    let encoded_query = url_encode(query);
    let mut url = format!(
        "{BASE_URL}/v5/place/text?keywords={encoded_query}&show_fields=business"
    );
    if !region.is_empty() {
        url.push_str(&format!("&region={}", url_encode(region)));
    }

    let body = do_get(&url)?;
    let resp: PlaceSearchResponse = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse place search response: {e}"))?;

    if resp.status != "1" {
        return Err(format!("Amap API error (status {})", resp.status));
    }

    let pois = resp.pois.unwrap_or_default();
    let formatted: Vec<serde_json::Value> = pois
        .into_iter()
        .filter_map(|p| {
            let name = p.name?;
            let mut entry = serde_json::json!({"name": name});
            if let Some(addr) = p.address {
                entry["address"] = serde_json::json!(addr);
            }
            if let Some(loc) = p.location {
                if let Ok((lng, lat)) = parse_location_str(&loc) {
                    entry["lng"] = serde_json::json!(lng);
                    entry["lat"] = serde_json::json!(lat);
                }
            }
            if let Some(t) = p.poi_type {
                entry["type"] = serde_json::json!(t);
            }
            if let Some(tel) = p.tel {
                entry["telephone"] = serde_json::json!(tel);
            }
            if let Some(id) = p.id {
                entry["id"] = serde_json::json!(id);
            }
            if let Some(biz) = p.business {
                entry["business"] = biz;
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

    // Amap uses lng,lat order
    let url = format!(
        "{BASE_URL}/v5/direction/driving?origin={lng},{lat}&destination={dest_lng},{dest_lat}"
    );

    let body = do_get(&url)?;
    let resp: RoutePlanResponse = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse route plan response: {e}"))?;

    if resp.status != "1" {
        return Err(format!("Amap API error (status {})", resp.status));
    }

    let route = resp.route.ok_or("No route plan result returned")?;
    let paths = route.paths.unwrap_or_default();

    let formatted: Vec<serde_json::Value> = paths
        .into_iter()
        .map(|p| {
            let steps: Vec<serde_json::Value> = p
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

            let mut path = serde_json::json!({"steps": steps});
            if let Some(d) = p.distance {
                path["total_distance_m"] = serde_json::json!(d);
            }
            if let Some(d) = p.duration {
                path["total_duration_s"] = serde_json::json!(d);
            }
            if let Some(s) = p.strategy {
                path["strategy"] = serde_json::json!(s);
            }
            path
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

/// Parse "lng,lat" string into (f64, f64).
fn parse_location_str(s: &str) -> Result<(f64, f64), String> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 2 {
        return Err(format!("Invalid location format: '{s}', expected 'lng,lat'"));
    }
    let lng: f64 = parts[0]
        .parse()
        .map_err(|e| format!("Invalid longitude '{}'': {e}", parts[0]))?;
    let lat: f64 = parts[1]
        .parse()
        .map_err(|e| format!("Invalid latitude '{}': {e}", parts[1]))?;
    Ok((lng, lat))
}

fn do_get(url: &str) -> Result<String, String> {
    let headers = serde_json::json!({
        "User-Agent": "IronClaw-Amap-Tool/0.1",
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
                    "Amap API error {} (attempt {}/{}). Retrying...",
                    resp.status, attempt, MAX_RETRIES
                ),
            );
            continue;
        }

        let body = String::from_utf8_lossy(&resp.body);
        return Err(format!("Amap API error (HTTP {}): {}", resp.status, body));
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
            "description": "Region to search in (for 'place_search')"
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

export!(AmapTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_encode() {
        assert_eq!(url_encode("hello"), "hello");
        assert_eq!(url_encode("上海"), "%E4%B8%8A%E6%B5%B7");
    }

    #[test]
    fn test_parse_location_str() {
        let (lng, lat) = parse_location_str("116.307484,40.056878").unwrap();
        assert!((lng - 116.307484).abs() < 0.0001);
        assert!((lat - 40.056878).abs() < 0.0001);
    }

    #[test]
    fn test_parse_location_str_invalid() {
        assert!(parse_location_str("invalid").is_err());
        assert!(parse_location_str("").is_err());
        assert!(parse_location_str("abc,def").is_err());
    }

    #[test]
    fn test_parse_geocode_response() {
        let json = r#"{
            "status": "1",
            "geocodes": [
                {
                    "formatted_address": "上海市浦东新区陆家嘴",
                    "location": "121.499740,31.239853",
                    "level": "兴趣点"
                }
            ]
        }"#;
        let resp: GeocodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, "1");
        let geocodes = resp.geocodes.unwrap();
        assert_eq!(geocodes.len(), 1);
        assert_eq!(geocodes[0].location.as_deref(), Some("121.499740,31.239853"));
    }

    #[test]
    fn test_parse_reverse_geocode_response() {
        let json = r#"{
            "status": "1",
            "regeocode": {
                "formatted_address": "上海市浦东新区世纪大道100号",
                "addressComponent": {"city": "上海市"}
            }
        }"#;
        let resp: ReverseGeocodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, "1");
        let regeo = resp.regeocode.unwrap();
        assert_eq!(
            regeo.formatted_address.as_deref(),
            Some("上海市浦东新区世纪大道100号")
        );
    }

    #[test]
    fn test_parse_place_search_response() {
        let json = r#"{
            "status": "1",
            "pois": [
                {
                    "name": "星巴克",
                    "address": "世纪大道100号",
                    "location": "121.499,31.239",
                    "type": "餐饮服务",
                    "tel": "021-12345678",
                    "id": "B001234"
                }
            ]
        }"#;
        let resp: PlaceSearchResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, "1");
        let pois = resp.pois.unwrap();
        assert_eq!(pois.len(), 1);
        assert_eq!(pois[0].name.as_deref(), Some("星巴克"));
    }

    #[test]
    fn test_parse_place_search_empty() {
        let json = r#"{"status": "1", "pois": []}"#;
        let resp: PlaceSearchResponse = serde_json::from_str(json).unwrap();
        assert!(resp.pois.unwrap().is_empty());
    }

    #[test]
    fn test_parse_route_plan_response() {
        let json = r#"{
            "status": "1",
            "route": {
                "paths": [
                    {
                        "distance": "12500",
                        "duration": "1800",
                        "strategy": "速度最快",
                        "steps": [
                            {
                                "instruction": "向东行驶500米",
                                "distance": "500",
                                "duration": "60"
                            }
                        ]
                    }
                ]
            }
        }"#;
        let resp: RoutePlanResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, "1");
        let route = resp.route.unwrap();
        let paths = route.paths.unwrap();
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].distance.as_deref(), Some("12500"));
        assert_eq!(paths[0].steps.len(), 1);
    }

    #[test]
    fn test_parse_route_plan_empty() {
        let json = r#"{"status": "1", "route": {"paths": []}}"#;
        let resp: RoutePlanResponse = serde_json::from_str(json).unwrap();
        assert!(resp.route.unwrap().paths.unwrap().is_empty());
    }
}
