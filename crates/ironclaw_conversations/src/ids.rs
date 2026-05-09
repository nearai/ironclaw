use serde::{Deserialize, Serialize};

use crate::InboundTurnError;

macro_rules! bounded_string_id {
    ($name:ident, $kind:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, InboundTurnError> {
                let value = value.into();
                validate_external_id($kind, &value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

bounded_string_id!(AdapterKind, "adapter_kind");
bounded_string_id!(AdapterInstallationId, "adapter_installation_id");
bounded_string_id!(ExternalEventId, "external_event_id");
bounded_string_id!(InboundMessageContentRef, "inbound_message_content_ref");

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct ExternalActorRef {
    kind: String,
    id: String,
}

impl ExternalActorRef {
    pub fn new(kind: impl Into<String>, id: impl Into<String>) -> Result<Self, InboundTurnError> {
        let kind = kind.into();
        let id = id.into();
        validate_external_id("external_actor_kind", &kind)?;
        validate_external_id("external_actor_id", &id)?;
        Ok(Self { kind, id })
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct ExternalConversationRef {
    space_id: Option<String>,
    conversation_id: String,
    thread_id: Option<String>,
    message_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct ExternalConversationIdentity {
    pub(crate) space_id: Option<String>,
    pub(crate) conversation_id: String,
    pub(crate) thread_id: Option<String>,
}

impl ExternalConversationIdentity {
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    pub(crate) fn conversation_fingerprint(&self) -> String {
        length_prefixed_fingerprint(&[
            self.space_id.as_deref().unwrap_or(""),
            &self.conversation_id,
            self.thread_id.as_deref().unwrap_or(""),
        ])
    }
}

impl ExternalConversationRef {
    pub fn new(
        space_id: Option<&str>,
        conversation_id: impl Into<String>,
        thread_id: Option<&str>,
        message_id: Option<&str>,
    ) -> Result<Self, InboundTurnError> {
        let space_id = space_id.map(str::to_string);
        let conversation_id = conversation_id.into();
        let thread_id = thread_id.map(str::to_string);
        let message_id = message_id.map(str::to_string);
        if let Some(value) = &space_id {
            validate_external_id("external_space_id", value)?;
        }
        validate_external_id("external_conversation_id", &conversation_id)?;
        if let Some(value) = &thread_id {
            validate_external_id("external_thread_id", value)?;
        }
        if let Some(value) = &message_id {
            validate_external_id("external_message_id", value)?;
        }
        Ok(Self {
            space_id,
            conversation_id,
            thread_id,
            message_id,
        })
    }

    pub fn space_id(&self) -> Option<&str> {
        self.space_id.as_deref()
    }

    pub fn conversation_id(&self) -> &str {
        &self.conversation_id
    }

    pub fn thread_id(&self) -> Option<&str> {
        self.thread_id.as_deref()
    }

    pub fn message_id(&self) -> Option<&str> {
        self.message_id.as_deref()
    }

    pub fn without_message_id(&self) -> Self {
        Self {
            space_id: self.space_id.clone(),
            conversation_id: self.conversation_id.clone(),
            thread_id: self.thread_id.clone(),
            message_id: None,
        }
    }

    pub(crate) fn identity(&self) -> ExternalConversationIdentity {
        ExternalConversationIdentity {
            space_id: self.space_id.clone(),
            conversation_id: self.conversation_id.clone(),
            thread_id: self.thread_id.clone(),
        }
    }

    pub fn conversation_fingerprint(&self) -> String {
        length_prefixed_fingerprint(&[
            self.space_id.as_deref().unwrap_or(""),
            &self.conversation_id,
            self.thread_id.as_deref().unwrap_or(""),
        ])
    }
}

fn length_prefixed_fingerprint(parts: &[&str]) -> String {
    let mut out = String::new();
    for part in parts {
        out.push_str(&part.len().to_string());
        out.push(':');
        out.push_str(part);
        out.push('|');
    }
    out
}

impl<'de> Deserialize<'de> for ExternalActorRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawExternalActorRef {
            kind: String,
            id: String,
        }

        let raw = RawExternalActorRef::deserialize(deserializer)?;
        Self::new(raw.kind, raw.id).map_err(serde::de::Error::custom)
    }
}

impl<'de> Deserialize<'de> for ExternalConversationIdentity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawExternalConversationIdentity {
            space_id: Option<String>,
            conversation_id: String,
            thread_id: Option<String>,
        }

        let raw = RawExternalConversationIdentity::deserialize(deserializer)?;
        ExternalConversationRef::new(
            raw.space_id.as_deref(),
            raw.conversation_id.as_str(),
            raw.thread_id.as_deref(),
            None,
        )
        .map(|conversation_ref| conversation_ref.identity())
        .map_err(serde::de::Error::custom)
    }
}

impl<'de> Deserialize<'de> for ExternalConversationRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawExternalConversationRef {
            space_id: Option<String>,
            conversation_id: String,
            thread_id: Option<String>,
            message_id: Option<String>,
        }

        let raw = RawExternalConversationRef::deserialize(deserializer)?;
        Self::new(
            raw.space_id.as_deref(),
            raw.conversation_id,
            raw.thread_id.as_deref(),
            raw.message_id.as_deref(),
        )
        .map_err(serde::de::Error::custom)
    }
}

fn validate_external_id(kind: &'static str, value: &str) -> Result<(), InboundTurnError> {
    if value.is_empty() {
        return Err(InboundTurnError::InvalidExternalRef {
            kind,
            reason: "must not be empty".to_string(),
        });
    }
    if value.len() > 512 {
        return Err(InboundTurnError::InvalidExternalRef {
            kind,
            reason: "must be at most 512 bytes".to_string(),
        });
    }
    if value.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(InboundTurnError::InvalidExternalRef {
            kind,
            reason: "must not contain NUL/control characters".to_string(),
        });
    }
    Ok(())
}
