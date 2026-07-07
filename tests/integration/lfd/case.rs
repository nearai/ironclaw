//! Case schema (SCHEMA.md §1) — the eval INPUT deserialized from
//! `$LFD_CASES/<case_id>.json`. Pinned runner code.

use serde::Deserialize;

fn default_http_stub_status() -> u16 {
    200
}

/// One eval case. Unknown top-level fields are rejected loudly so a case
/// written against a future schema version fails as `status: "error"` instead
/// of silently dropping constraints.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub schema_version: u32,
    pub case_id: String,
    pub profile: String,
    /// Human-readable label; carried for schema completeness, not read by the
    /// runner.
    #[serde(default)]
    #[allow(dead_code)]
    pub title: String,
    #[serde(default)]
    pub setup: CaseSetup,
    #[serde(default)]
    pub llm_script: Vec<ScriptTurn>,
    #[serde(default)]
    pub inbound: Vec<InboundEntry>,
    #[serde(default)]
    pub state_queries: Vec<StateQuery>,
    #[serde(default)]
    pub live: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseSetup {
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub secrets: Vec<CaseSecret>,
    #[serde(default)]
    pub memory_docs: Vec<serde_json::Value>,
    #[serde(default)]
    pub triggers: Vec<serde_json::Value>,
    #[serde(default)]
    pub http_stubs: Vec<HttpStub>,
    /// Profile-specific setup; schema owned by the profile (SCHEMA.md §1).
    #[serde(default)]
    pub profile_extra: serde_json::Value,
}

impl CaseSetup {
    /// `true` when `profile_extra` carries actual content a profile must
    /// interpret (i.e. not the schema's `{}`/omitted default).
    pub fn has_profile_extra(&self) -> bool {
        match &self.profile_extra {
            serde_json::Value::Null => false,
            serde_json::Value::Object(map) => !map.is_empty(),
            _ => true,
        }
    }
}

/// An injected secret. `value` literals feed the leak scan regardless of
/// whether the profile wires the secret into a store.
#[derive(Debug, Clone, Deserialize)]
pub struct CaseSecret {
    /// Backend secret identity; read by profiles that wire a secret store
    /// (none yet), kept for schema completeness.
    #[allow(dead_code)]
    pub credential_name: String,
    pub value: String,
}

/// A scripted HTTP response for the case's tool egress. `key` is matched as a
/// URL substring by the harness's keyed HTTP matcher.
#[derive(Debug, Clone, Deserialize)]
pub struct HttpStub {
    pub key: String,
    #[serde(default = "default_http_stub_status")]
    pub status: u16,
    #[serde(default)]
    pub body: serde_json::Value,
}

/// One scripted model turn (FIFO across the whole case).
#[derive(Debug, Clone, Deserialize)]
pub struct ScriptTurn {
    /// 1-based turn label; documentation only — the script is a FIFO, so
    /// steps are consumed in file order regardless of this value.
    #[serde(default)]
    #[allow(dead_code)]
    pub turn: u32,
    pub steps: Vec<ScriptStep>,
}

/// One scripted model call: a tool step maps to
/// `RebornScriptedReply::tool_call`, a text step to `RebornScriptedReply::text`.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ScriptStep {
    Tool {
        tool: String,
        #[serde(default)]
        params: serde_json::Value,
    },
    Text {
        text: String,
    },
}

/// One inbound message driving one turn.
#[derive(Debug, Clone, Deserialize)]
pub struct InboundEntry {
    #[serde(default)]
    pub channel: String,
    pub payload: serde_json::Value,
}

/// A declarative post-scenario read against persisted state (SCHEMA.md §1).
#[derive(Debug, Clone, Deserialize)]
pub struct StateQuery {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub params: serde_json::Value,
}
