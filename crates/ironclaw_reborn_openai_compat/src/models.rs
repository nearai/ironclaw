use serde::{Deserialize, Serialize};

/// A single entry in the OpenAI-compatible `GET /v1/models` listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAiModelObject {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub owned_by: String,
}

/// The OpenAI-compatible `GET /v1/models` list envelope (`{ object, data }`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAiModelListResponse {
    pub object: String,
    pub data: Vec<OpenAiModelObject>,
}
