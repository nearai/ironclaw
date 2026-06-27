use serde::de;
use serde::{Deserialize, Deserializer, Serialize};

use crate::{OpenAiCompatError, OpenAiCompatErrorCode, OpenAiCompatErrorKind};

use crate::OpenAiResponseId;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenAiResponsesCreateRequest {
    pub model: String,
    pub input: OpenAiResponsesInput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<OpenAiResponseId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<serde_json::Value>,
    /// IronClaw extension: structured context injected into the submitted turn.
    #[serde(default, alias = "context", skip_serializing_if = "Option::is_none")]
    pub x_context: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OpenAiResponsesInput {
    Text(String),
    Items(Vec<OpenAiResponsesInputItem>),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OpenAiResponsesInputItem {
    Message {
        role: OpenAiResponsesMessageRole,
        content: serde_json::Value,
    },
    FunctionCall {
        call_id: String,
        name: String,
        arguments: String,
    },
    FunctionCallOutput {
        call_id: String,
        output: serde_json::Value,
    },
}

impl<'de> Deserialize<'de> for OpenAiResponsesInputItem {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let object = value
            .as_object()
            .ok_or_else(|| de::Error::custom("responses input item must be an object"))?;
        match object.get("type").and_then(serde_json::Value::as_str) {
            Some("message") => {
                #[derive(Deserialize)]
                struct MessageWire {
                    role: OpenAiResponsesMessageRole,
                    content: serde_json::Value,
                }
                let wire = MessageWire::deserialize(value).map_err(de::Error::custom)?;
                Ok(Self::Message {
                    role: wire.role,
                    content: wire.content,
                })
            }
            None if object.contains_key("role") => {
                #[derive(Deserialize)]
                struct MessageWire {
                    role: OpenAiResponsesMessageRole,
                    content: serde_json::Value,
                }
                let wire = MessageWire::deserialize(value).map_err(de::Error::custom)?;
                Ok(Self::Message {
                    role: wire.role,
                    content: wire.content,
                })
            }
            Some("function_call") => {
                #[derive(Deserialize)]
                struct FunctionCallWire {
                    call_id: String,
                    name: String,
                    arguments: String,
                }
                let wire = FunctionCallWire::deserialize(value).map_err(de::Error::custom)?;
                Ok(Self::FunctionCall {
                    call_id: wire.call_id,
                    name: wire.name,
                    arguments: wire.arguments,
                })
            }
            Some("function_call_output") => {
                #[derive(Deserialize)]
                struct FunctionCallOutputWire {
                    call_id: String,
                    output: serde_json::Value,
                }
                let wire = FunctionCallOutputWire::deserialize(value).map_err(de::Error::custom)?;
                Ok(Self::FunctionCallOutput {
                    call_id: wire.call_id,
                    output: wire.output,
                })
            }
            Some(other) => Err(de::Error::custom(format!(
                "unsupported responses input item type: {other}"
            ))),
            None => Err(de::Error::custom(
                "responses input item is missing type or role",
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpenAiResponsesMessageRole {
    System,
    Developer,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenAiResponseObject {
    pub id: OpenAiResponseId,
    pub object: String,
    pub created_at: u64,
    pub status: OpenAiResponseStatus,
    pub model: String,
    #[serde(default)]
    pub output: Vec<OpenAiResponseOutputItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<OpenAiResponseErrorObject>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incomplete_details: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<OpenAiResponseUsage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAiResponseUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiResponseStatus {
    Queued,
    InProgress,
    Completed,
    Failed,
    Cancelled,
    Incomplete,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum OpenAiResponseOutputItem {
    Message {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<OpenAiResponseOutputItemStatus>,
        role: OpenAiResponsesMessageRole,
        content: serde_json::Value,
    },
    FunctionCall {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<OpenAiResponseOutputItemStatus>,
        call_id: String,
        name: String,
        arguments: String,
    },
    FunctionCallOutput {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<OpenAiResponseOutputItemStatus>,
        call_id: String,
        output: serde_json::Value,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiResponseOutputItemStatus {
    InProgress,
    Completed,
    Incomplete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenAiResponseErrorObject {
    code: OpenAiCompatErrorCode,
    message: String,
}

impl OpenAiResponseErrorObject {
    pub fn from_kind(kind: OpenAiCompatErrorKind) -> Self {
        Self::from_compat_error(&OpenAiCompatError::from_kind(kind, None))
    }

    pub fn from_compat_error(error: &OpenAiCompatError) -> Self {
        let code = error.code().unwrap_or(OpenAiCompatErrorCode::InternalError);
        Self {
            code,
            message: code.sanitized_message().to_string(),
        }
    }

    pub fn code(&self) -> OpenAiCompatErrorCode {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenAiResponseErrorObjectWire {
    code: OpenAiCompatErrorCode,
    message: String,
}

impl<'de> Deserialize<'de> for OpenAiResponseErrorObject {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = OpenAiResponseErrorObjectWire::deserialize(deserializer)?;
        let expected = wire.code.sanitized_message();
        if wire.message != expected {
            return Err(de::Error::custom(
                "response error message must match sanitized error code",
            ));
        }
        Ok(Self {
            code: wire.code,
            message: expected.to_string(),
        })
    }
}
