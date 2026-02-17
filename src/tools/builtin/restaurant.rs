//! Restaurant reservation tool.

use async_trait::async_trait;

use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolError, ToolOutput};

/// Tool for restaurant reservations (OpenTable, Resy, etc.).
pub struct RestaurantTool {
    // TODO: Add reservation API clients
}

impl RestaurantTool {
    /// Create a new restaurant tool.
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for RestaurantTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for RestaurantTool {
    fn name(&self) -> &str {
        "restaurant"
    }

    fn description(&self) -> &str {
        "Search restaurants, check availability, and make reservations via OpenTable, Resy, etc."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["search", "check_availability", "make_reservation", "cancel_reservation", "get_reservation"],
                    "description": "The restaurant action to perform"
                },
                "query": {
                    "type": "string",
                    "description": "Search query (cuisine type, restaurant name, etc.)"
                },
                "location": {
                    "type": "object",
                    "properties": {
                        "city": { "type": "string" },
                        "neighborhood": { "type": "string" },
                        "latitude": { "type": "number" },
                        "longitude": { "type": "number" }
                    },
                    "description": "Location to search near"
                },
                "date": {
                    "type": "string",
                    "description": "Reservation date (YYYY-MM-DD)"
                },
                "time": {
                    "type": "string",
                    "description": "Preferred time (HH:MM)"
                },
                "party_size": {
                    "type": "integer",
                    "description": "Number of guests"
                },
                "restaurant_id": {
                    "type": "string",
                    "description": "Restaurant ID (for check_availability, make_reservation)"
                },
                "reservation_id": {
                    "type": "string",
                    "description": "Reservation ID (for cancel_reservation, get_reservation)"
                },
                "guest_name": {
                    "type": "string",
                    "description": "Name for the reservation"
                },
                "guest_phone": {
                    "type": "string",
                    "description": "Phone number for the reservation"
                },
                "guest_email": {
                    "type": "string",
                    "description": "Email for the reservation"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidParameters("missing 'action' parameter".to_string())
            })?;

        // TODO: Implement actual restaurant reservation API integrations
        let result = match action {
            "search" => {
                let query = params.get("query").and_then(|v| v.as_str()).unwrap_or("");

                serde_json::json!({
                    "query": query,
                    "restaurants": [],
                    "message": "Restaurant integration not yet implemented"
                })
            }
            "check_availability" => {
                let restaurant_id = params
                    .get("restaurant_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ToolError::InvalidParameters(
                            "missing 'restaurant_id' parameter".to_string(),
                        )
                    })?;

                serde_json::json!({
                    "restaurant_id": restaurant_id,
                    "available_times": [],
                    "message": "Restaurant integration not yet implemented"
                })
            }
            "make_reservation" => {
                serde_json::json!({
                    "success": false,
                    "message": "Restaurant integration not yet implemented"
                })
            }
            "cancel_reservation" => {
                serde_json::json!({
                    "cancelled": false,
                    "message": "Restaurant integration not yet implemented"
                })
            }
            "get_reservation" => {
                let reservation_id = params.get("reservation_id").and_then(|v| v.as_str());

                serde_json::json!({
                    "reservation_id": reservation_id,
                    "found": false,
                    "message": "Restaurant integration not yet implemented"
                })
            }
            _ => {
                return Err(ToolError::InvalidParameters(format!(
                    "unknown action: {}",
                    action
                )));
            }
        };

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        true // External restaurant data
    }
}
