//! ANP Agent Description preview generation.

use serde::{Deserialize, Serialize};

use crate::did::InstanceIdentity;

/// Minimal ANP-compatible agent description returned by the preview endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDescription {
    #[serde(rename = "protocolType")]
    pub protocol_type: String,
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    #[serde(rename = "type")]
    pub document_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub name: String,
    pub did: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interfaces: Vec<AgentInterface>,
}

/// Minimal ANP interface entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInterface {
    #[serde(rename = "type")]
    pub interface_type: String,
    pub protocol: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "humanAuthorization", skip_serializing_if = "Option::is_none")]
    pub human_authorization: Option<bool>,
}

/// Build a conservative ANP Agent Description preview for the current instance.
pub fn build_agent_description_preview(
    agent_name: &str,
    identity: &InstanceIdentity,
    tool_count: usize,
    has_openai_compat: bool,
) -> AgentDescription {
    let mut interfaces = vec![AgentInterface {
        interface_type: "NaturalLanguageInterface".to_string(),
        protocol: "HTTP+JSON".to_string(),
        version: Some("preview".to_string()),
        url: Some("/api/chat/send".to_string()),
        description: Some(
            "Authenticated local gateway endpoint for natural language interaction.".to_string(),
        ),
        human_authorization: Some(false),
    }];

    if has_openai_compat {
        interfaces.push(AgentInterface {
            interface_type: "StructuredInterface".to_string(),
            protocol: "OpenAI-Compatible".to_string(),
            version: Some("v1".to_string()),
            url: Some("/v1/chat/completions".to_string()),
            description: Some(
                "Authenticated local OpenAI-compatible chat completion endpoint.".to_string(),
            ),
            human_authorization: Some(false),
        });
    }

    AgentDescription {
        protocol_type: "ANP".to_string(),
        protocol_version: "1.0.0".to_string(),
        document_type: "AgentDescription".to_string(),
        url: None,
        name: agent_name.to_string(),
        did: identity.did().to_string(),
        description: Some(format!(
            "{agent_name} is an IronClaw agent with a stable instance DID. This local preview advertises {} registered tool(s).",
            tool_count
        )),
        created: Some(identity.created_at().to_rfc3339()),
        interfaces,
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use crate::did::InstanceIdentity;

    use super::*;

    #[test]
    fn preview_uses_identity_and_agent_name() {
        let created_at = Utc
            .with_ymd_and_hms(2025, 1, 1, 0, 0, 0)
            .single()
            .expect("valid timestamp");
        let identity = InstanceIdentity::from_secret_key([9u8; 32], created_at);
        let preview = build_agent_description_preview("Test Agent", &identity, 3, true);

        assert_eq!(preview.protocol_type, "ANP");
        assert_eq!(preview.document_type, "AgentDescription");
        assert_eq!(preview.name, "Test Agent");
        assert_eq!(preview.did, identity.did());
        assert_eq!(preview.interfaces.len(), 2);
        assert!(
            preview
                .description
                .as_deref()
                .unwrap()
                .contains("3 registered tool")
        );
    }
}
