use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "action")]
pub enum AtlasNotionAction {
    #[serde(rename = "create_page")]
    CreatePage {
        database_id: String,
        properties: serde_json::Value,
    },
    #[serde(rename = "query_database")]
    QueryDatabase {
        database_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        filter: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        sorts: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        start_cursor: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        page_size: Option<u32>,
    },
    #[serde(rename = "update_page")]
    UpdatePage {
        page_id: String,
        properties: serde_json::Value,
    },
    #[serde(rename = "get_page")]
    GetPage {
        page_id: String,
    },
    #[serde(rename = "search")]
    Search {
        #[serde(skip_serializing_if = "Option::is_none")]
        query: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        start_cursor: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        page_size: Option<u32>,
    },
}
