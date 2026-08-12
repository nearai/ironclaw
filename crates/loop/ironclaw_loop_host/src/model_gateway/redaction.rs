//! Final deterministic redaction for provider-bound model requests.

use std::collections::{HashMap, HashSet};

use ironclaw_llm::{
    ChatMessage, CompletionRequest, ContentPart, ReasoningDetail, ToolCompletionRequest,
    ToolDefinition,
};
use ironclaw_safety::{redact_model_input_text, redact_model_input_url};

const REDACTED_SECRET: &str = "[REDACTED_SECRET]";

pub(super) fn redact_completion_request(request: &mut CompletionRequest) -> usize {
    redact_chat_messages(&mut request.messages)
        .saturating_add(redact_optional_strings(&mut request.stop_sequences))
}

pub(super) fn redact_tool_completion_request(request: &mut ToolCompletionRequest) -> usize {
    redact_chat_messages(&mut request.messages)
        .saturating_add(redact_tool_definitions(&mut request.tools))
        .saturating_add(redact_optional_strings(&mut request.stop_sequences))
}

fn redact_optional_strings(values: &mut Option<Vec<String>>) -> usize {
    values.as_mut().map_or(0, |values| {
        values.iter_mut().fold(0usize, |count, value| {
            count.saturating_add(redact_string(value))
        })
    })
}

fn redact_chat_messages(messages: &mut [ChatMessage]) -> usize {
    messages.iter_mut().fold(0usize, |count, message| {
        count.saturating_add(redact_chat_message(message))
    })
}

fn redact_chat_message(message: &mut ChatMessage) -> usize {
    let mut count = redact_string(&mut message.content);
    for part in &mut message.content_parts {
        match part {
            ContentPart::Text { text } => {
                count = count.saturating_add(redact_string(text));
            }
            ContentPart::ImageUrl { image_url } => {
                let redaction = redact_model_input_url(&image_url.url);
                count = count.saturating_add(redaction.redaction_count());
                if redaction.was_modified() {
                    image_url.url = redaction.into_text();
                }
            }
        }
    }
    if let Some(reasoning) = message.reasoning.as_mut() {
        count = count.saturating_add(redact_string(reasoning));
    }
    if let Some(details) = message.reasoning_details.as_mut() {
        for detail in &mut details.content {
            match detail {
                ReasoningDetail::Text { text, .. } | ReasoningDetail::Summary(text) => {
                    count = count.saturating_add(redact_string(text));
                }
                ReasoningDetail::Encrypted(_) | ReasoningDetail::Redacted { .. } => {}
            }
        }
    }
    if let Some(tool_calls) = message.tool_calls.as_mut() {
        for tool_call in tool_calls {
            count = count.saturating_add(redact_json_string_values(&mut tool_call.arguments));
            if let Some(reasoning) = tool_call.reasoning.as_mut() {
                count = count.saturating_add(redact_string(reasoning));
            }
            if let Some(parse_error) = tool_call.arguments_parse_error.as_mut() {
                count = count.saturating_add(redact_string(parse_error));
            }
        }
    }
    count
}

pub(super) fn redact_tool_definitions(definitions: &mut [ToolDefinition]) -> usize {
    definitions.iter_mut().fold(0usize, |count, definition| {
        count
            .saturating_add(redact_string(&mut definition.description))
            .saturating_add(redact_json_schema(&mut definition.parameters))
    })
}

fn redact_json_string_values(value: &mut serde_json::Value) -> usize {
    redact_json_value(value, JsonRedactionContext::Ordinary)
}

fn redact_json_schema(value: &mut serde_json::Value) -> usize {
    redact_json_value(value, JsonRedactionContext::Schema)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum JsonRedactionContext {
    Ordinary,
    Schema,
}

fn redact_json_value(value: &mut serde_json::Value, context: JsonRedactionContext) -> usize {
    match value {
        serde_json::Value::String(text) => redact_string(text),
        serde_json::Value::Array(values) => values.iter_mut().fold(0usize, |count, value| {
            count.saturating_add(redact_json_value(value, context))
        }),
        serde_json::Value::Object(values) => redact_json_object(values, context).0,
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => 0,
    }
}

fn redact_json_object(
    values: &mut serde_json::Map<String, serde_json::Value>,
    context: JsonRedactionContext,
) -> (usize, HashMap<String, String>) {
    let original = std::mem::take(values);
    let mut entries = original
        .into_iter()
        .map(|(original_key, value)| {
            let mut redacted_key = original_key.clone();
            let key_redaction_count = redact_string(&mut redacted_key);
            (original_key, redacted_key, key_redaction_count, value)
        })
        .collect::<Vec<_>>();

    // Reserve unchanged names first. A pre-existing placeholder-shaped key
    // must not be displaced by a secret-bearing key that redacts to the same
    // text, and no member may be lost to Map::insert replacement.
    let mut used_keys = entries
        .iter()
        .filter(|(_, _, key_redaction_count, _)| *key_redaction_count == 0)
        .map(|(_, redacted_key, _, _)| redacted_key.clone())
        .collect::<HashSet<_>>();
    for (_, redacted_key, key_redaction_count, _) in &mut entries {
        if *key_redaction_count > 0 {
            *redacted_key = collision_safe_json_key(redacted_key, &mut used_keys);
        }
    }

    let key_mapping = entries
        .iter()
        .map(|(original_key, redacted_key, _, _)| (original_key.clone(), redacted_key.clone()))
        .collect::<HashMap<_, _>>();
    let mut redaction_count = entries.iter().fold(0usize, |count, (_, _, key_count, _)| {
        count.saturating_add(*key_count)
    });

    // JSON Schema's `required` entries refer to keys in the sibling
    // `properties` object. Capture that object's exact collision-safe mapping
    // before visiting the references so the provider receives a valid schema.
    let mut property_key_mapping = None;
    if context == JsonRedactionContext::Schema
        && let Some((_, _, _, properties)) = entries
            .iter_mut()
            .find(|(original_key, _, _, _)| original_key == "properties")
    {
        let (count, mapping) = match properties {
            serde_json::Value::Object(properties) => {
                redact_json_object(properties, JsonRedactionContext::Schema)
            }
            _ => (redact_json_schema(properties), HashMap::new()),
        };
        redaction_count = redaction_count.saturating_add(count);
        property_key_mapping = Some(mapping);
    }

    for (original_key, redacted_key, _, mut value) in entries {
        let properties_were_preprocessed =
            context == JsonRedactionContext::Schema && original_key == "properties";
        if !properties_were_preprocessed {
            let count = if context == JsonRedactionContext::Schema && original_key == "required" {
                match property_key_mapping.as_ref() {
                    Some(mapping) => redact_json_property_references(&mut value, mapping),
                    None => redact_json_schema(&mut value),
                }
            } else if is_sensitive_json_key(&original_key) {
                redact_sensitive_json_value(&mut value, context)
            } else {
                redact_json_value(&mut value, context)
            };
            redaction_count = redaction_count.saturating_add(count);
        }
        values.insert(redacted_key, value);
    }

    (redaction_count, key_mapping)
}

fn is_sensitive_json_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    matches!(
        normalized.as_str(),
        "password"
            | "passwd"
            | "authorization"
            | "accesstoken"
            | "refreshtoken"
            | "token"
            | "apikey"
            | "apisecret"
            | "clientsecret"
            | "secret"
            | "secretkey"
            | "secrettoken"
            | "sharedsecret"
            | "credential"
            | "privatekey"
    )
}

fn redact_sensitive_json_value(
    value: &mut serde_json::Value,
    context: JsonRedactionContext,
) -> usize {
    match value {
        serde_json::Value::String(text) if text == REDACTED_SECRET => 0,
        serde_json::Value::String(text) => {
            *text = REDACTED_SECRET.to_string();
            1
        }
        serde_json::Value::Array(values) => values.iter_mut().fold(0usize, |count, value| {
            count.saturating_add(redact_sensitive_json_value(value, context))
        }),
        serde_json::Value::Object(values) if context == JsonRedactionContext::Schema => {
            values.iter_mut().fold(0usize, |count, (key, value)| {
                let field_count =
                    if is_schema_secret_value_field(key) || is_schema_composition_field(key) {
                        redact_sensitive_json_value(value, context)
                    } else {
                        redact_json_value(value, context)
                    };
                count.saturating_add(field_count)
            })
        }
        serde_json::Value::Object(values) => values.values_mut().fold(0usize, |count, value| {
            count.saturating_add(redact_sensitive_json_value(value, context))
        }),
        serde_json::Value::Null => 0,
        serde_json::Value::Bool(boolean) => {
            let modified = *boolean;
            *boolean = false;
            usize::from(modified)
        }
        serde_json::Value::Number(number) => {
            let redacted = serde_json::Number::from(0);
            if *number == redacted {
                0
            } else {
                *number = redacted;
                1
            }
        }
    }
}

fn is_schema_secret_value_field(key: &str) -> bool {
    matches!(key, "const" | "default" | "enum" | "example" | "examples")
}

fn is_schema_composition_field(key: &str) -> bool {
    matches!(
        key,
        "allOf"
            | "anyOf"
            | "oneOf"
            | "items"
            | "prefixItems"
            | "contains"
            | "not"
            | "if"
            | "then"
            | "else"
    )
}

fn collision_safe_json_key(base: &str, used_keys: &mut HashSet<String>) -> String {
    if used_keys.insert(base.to_string()) {
        return base.to_string();
    }
    let mut discriminator = 2usize;
    loop {
        let candidate = format!("{base}#{discriminator}");
        if used_keys.insert(candidate.clone()) {
            return candidate;
        }
        discriminator = discriminator.saturating_add(1);
    }
}

fn redact_json_property_references(
    value: &mut serde_json::Value,
    key_mapping: &HashMap<String, String>,
) -> usize {
    match value {
        serde_json::Value::Array(values) => values.iter_mut().fold(0usize, |count, value| {
            let field_count = match value {
                serde_json::Value::String(reference) => {
                    let original = reference.clone();
                    let redaction = redact_model_input_text(&original);
                    let finding_count = redaction.redaction_count();
                    *reference = key_mapping
                        .get(&original)
                        .cloned()
                        .unwrap_or_else(|| redaction.into_text());
                    finding_count
                }
                _ => redact_json_schema(value),
            };
            count.saturating_add(field_count)
        }),
        _ => redact_json_schema(value),
    }
}

fn redact_string(value: &mut String) -> usize {
    let redaction = redact_model_input_text(value);
    let count = redaction.redaction_count();
    if count > 0 {
        *value = redaction.into_text();
    }
    count
}

#[cfg(test)]
mod tests {
    use ironclaw_llm::{ChatMessage, CompletionRequest, ContentPart, ImageUrl};

    use super::{redact_completion_request, redact_json_schema, redact_json_string_values};

    #[test]
    fn sensitive_json_keys_redact_arguments_and_schema_defaults() {
        let mut arguments = serde_json::json!({
            "password": "hunter2",
            "nested": {"Authorization": "Bearer weak secret"},
            "marker": "visible",
        });
        let argument_count = redact_json_string_values(&mut arguments);

        assert_eq!(argument_count, 2);
        assert_eq!(arguments["password"], "[REDACTED_SECRET]");
        assert_eq!(arguments["nested"]["Authorization"], "[REDACTED_SECRET]");
        assert_eq!(arguments["marker"], "visible");

        let mut schema = serde_json::json!({
            "type": "object",
            "properties": {
                "password": {
                    "type": "string",
                    "description": "Account password supplied by the user",
                    "default": "schema default secret",
                    "examples": ["first example secret", "second example secret"],
                    "anyOf": [
                        {"type": "string", "const": "nested schema secret"}
                    ]
                }
            }
        });
        let schema_count = redact_json_schema(&mut schema);

        assert_eq!(schema_count, 4);
        assert_eq!(schema["properties"]["password"]["type"], "string");
        assert_eq!(
            schema["properties"]["password"]["description"],
            "Account password supplied by the user"
        );
        assert_eq!(
            schema["properties"]["password"]["default"],
            "[REDACTED_SECRET]"
        );
        assert_eq!(
            schema["properties"]["password"]["examples"],
            serde_json::json!(["[REDACTED_SECRET]", "[REDACTED_SECRET]"])
        );
        assert_eq!(
            schema["properties"]["password"]["anyOf"][0]["const"],
            "[REDACTED_SECRET]"
        );
    }

    #[test]
    fn provider_request_redacts_remote_image_url_credentials_and_preserves_data_url() {
        let secret = "remote-image-query-secret";
        let data_url = "data:image/png;base64,cGFzc3dvcmQ6IGxldG1laW4=";
        let mut request = CompletionRequest::new(vec![ChatMessage::user_with_parts(
            "inspect images",
            vec![
                ContentPart::ImageUrl {
                    image_url: ImageUrl {
                        url: format!(
                            "https://example.test/users/42/avatar.png?size=large&token={secret}#access_token=fragment-secret&state=visible"
                        ),
                        detail: None,
                    },
                },
                ContentPart::ImageUrl {
                    image_url: ImageUrl {
                        url: data_url.to_string(),
                        detail: None,
                    },
                },
            ],
        )]);

        let count = redact_completion_request(&mut request);

        assert_eq!(count, 2);
        let ContentPart::ImageUrl { image_url: remote } = &request.messages[0].content_parts[0]
        else {
            panic!("first part must remain an image URL");
        };
        assert!(!remote.url.contains(secret));
        assert!(!remote.url.contains("fragment-secret"));
        assert!(remote.url.contains("/users/42/avatar.png"));
        assert!(remote.url.contains("size=large"));
        assert!(remote.url.contains("state=visible"));
        assert!(remote.url.contains("REDACTED_SECRET"));
        let ContentPart::ImageUrl { image_url: data } = &request.messages[0].content_parts[1]
        else {
            panic!("second part must remain an image URL");
        };
        assert_eq!(data.url, data_url);
    }

    #[test]
    fn ordinary_sensitive_object_is_not_misclassified_as_schema_and_scalars_keep_types() {
        let mut arguments = serde_json::json!({
            "credential": {
                "type": "basic",
                "description": "prod",
                "value": "hunter2"
            },
            "refresh_token": 123456,
            "secret": true,
            "marker": "visible"
        });

        redact_json_string_values(&mut arguments);

        assert!(!arguments.to_string().contains("hunter2"));
        assert_eq!(arguments["credential"]["value"], "[REDACTED_SECRET]");
        assert!(arguments["refresh_token"].is_number());
        assert_eq!(arguments["refresh_token"], 0);
        assert!(arguments["secret"].is_boolean());
        assert_eq!(arguments["secret"], false);
        assert_eq!(arguments["marker"], "visible");
    }
}
