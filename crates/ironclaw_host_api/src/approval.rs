//! Approval contracts for user-mediated authority.
//!
//! Approval is a scoped grant to continue a specific action, not a vague
//! confirmation. [`ApprovalRequest`] carries the exact action that needs a
//! decision and may optionally describe a reusable [`ApprovalScope`] such as a
//! capability, path prefix, or network target. Matching must be exact or
//! policy-defined by the host; callers must not infer broader authority from a
//! one-off approval.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    Action, ApprovalRequestId, CapabilityId, CorrelationId, HostApiError, NetworkTargetPattern,
    Principal, ResourceEstimate, ResourceScope, ScopedPath, Timestamp,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: ApprovalRequestId,
    pub correlation_id: CorrelationId,
    pub requested_by: Principal,
    pub action: Box<Action>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_fingerprint: Option<InvocationFingerprint>,
    pub reason: String,
    pub reusable_scope: Option<ApprovalScope>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InvocationFingerprint(String);

impl InvocationFingerprint {
    pub fn for_dispatch(
        scope: &ResourceScope,
        capability: &CapabilityId,
        estimate: &ResourceEstimate,
        input: &serde_json::Value,
    ) -> Result<Self, HostApiError> {
        Self::for_action("dispatch", scope, capability, estimate, input)
    }

    pub fn for_spawn(
        scope: &ResourceScope,
        capability: &CapabilityId,
        estimate: &ResourceEstimate,
        input: &serde_json::Value,
    ) -> Result<Self, HostApiError> {
        Self::for_action("spawn_capability", scope, capability, estimate, input)
    }

    fn for_action(
        kind: &'static str,
        scope: &ResourceScope,
        capability: &CapabilityId,
        estimate: &ResourceEstimate,
        input: &serde_json::Value,
    ) -> Result<Self, HostApiError> {
        #[derive(Serialize)]
        struct Payload<'a> {
            version: u8,
            kind: &'static str,
            scope: &'a ResourceScope,
            capability: &'a CapabilityId,
            estimate: &'a ResourceEstimate,
            input: &'a serde_json::Value,
        }

        let canonical_input = canonical_json(input)?;
        let payload = Payload {
            version: 1,
            kind,
            scope,
            capability,
            estimate,
            input: &canonical_input,
        };
        let bytes = serde_json::to_vec(&payload)
            .map_err(|error| HostApiError::invariant(error.to_string()))?;
        let digest = Sha256::digest(bytes);
        Ok(Self(format!("sha256:{}", to_lower_hex(&digest))))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

const MAX_CANONICAL_JSON_DEPTH: usize = 64;

fn canonical_json(value: &serde_json::Value) -> Result<serde_json::Value, HostApiError> {
    canonical_json_at_depth(value, 0)
}

fn canonical_json_at_depth(
    value: &serde_json::Value,
    depth: usize,
) -> Result<serde_json::Value, HostApiError> {
    if depth > MAX_CANONICAL_JSON_DEPTH {
        return Err(HostApiError::invariant(
            "canonical_json: max depth exceeded",
        ));
    }

    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .map(|item| canonical_json_at_depth(item, depth + 1))
            .collect::<Result<Vec<_>, _>>()
            .map(serde_json::Value::Array),
        serde_json::Value::Object(map) => {
            let mut entries = map.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(key, _)| *key);
            let mut canonical = serde_json::Map::new();
            for (key, value) in entries {
                canonical.insert(key.clone(), canonical_json_at_depth(value, depth + 1)?);
            }
            Ok(serde_json::Value::Object(canonical))
        }
        _ => Ok(value.clone()),
    }
}

fn to_lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalScope {
    pub principal: Principal,
    pub action_pattern: ActionPattern,
    pub expires_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ActionPattern {
    ExactAction {
        action: Box<Action>,
    },
    Capability {
        capability: CapabilityId,
    },
    PathPrefix {
        action_kind: FileActionKind,
        prefix: ScopedPath,
    },
    NetworkTarget {
        target: NetworkTargetPattern,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileActionKind {
    Read,
    List,
    Write,
    Delete,
}
