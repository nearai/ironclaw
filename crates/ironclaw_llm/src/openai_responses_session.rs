//! Provider-private request planning for a retained OpenAI Responses transport.
//!
//! The durable transcript remains authoritative. This module retains only a
//! provider response ID and structural fingerprints of acknowledged items.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use uuid::Uuid;

const MAX_RESPONSES_SESSIONS: usize = 256;
const SESSION_METADATA_KEY: &str = "agent_loop_session_id";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ResponsesSessionKey(Uuid);

impl ResponsesSessionKey {
    fn from_metadata(metadata: &HashMap<String, String>) -> Option<Self> {
        Uuid::parse_str(metadata.get(SESSION_METADATA_KEY)?)
            .ok()
            .map(Self)
    }
}

pub(crate) struct ResponsesSessionRegistry {
    inner: Mutex<ResponsesSessionRegistryInner>,
}

struct ResponsesSessionRegistryInner {
    sessions: HashMap<ResponsesSessionKey, Arc<Mutex<ResponsesSessionState>>>,
    insertion_order: VecDeque<ResponsesSessionKey>,
}

impl ResponsesSessionRegistry {
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(ResponsesSessionRegistryInner {
                sessions: HashMap::new(),
                insertion_order: VecDeque::new(),
            }),
        }
    }

    pub(crate) async fn session_for_metadata(
        &self,
        metadata: &HashMap<String, String>,
    ) -> Option<Arc<Mutex<ResponsesSessionState>>> {
        let key = ResponsesSessionKey::from_metadata(metadata)?;
        let mut inner = self.inner.lock().await;
        if let Some(session) = inner.sessions.get(&key) {
            return Some(Arc::clone(session));
        }

        let mut scanned = 0;
        while inner.sessions.len() >= MAX_RESPONSES_SESSIONS
            && scanned < inner.insertion_order.len()
        {
            let Some(oldest) = inner.insertion_order.pop_front() else {
                break;
            };
            let idle = inner
                .sessions
                .get(&oldest)
                .is_some_and(|session| Arc::strong_count(session) == 1);
            if idle {
                inner.sessions.remove(&oldest);
            } else {
                inner.insertion_order.push_back(oldest);
                scanned += 1;
            }
        }
        if inner.sessions.len() >= MAX_RESPONSES_SESSIONS {
            return None;
        }

        let session = Arc::new(Mutex::new(ResponsesSessionState::default()));
        inner.insertion_order.push_back(key);
        inner.sessions.insert(key, Arc::clone(&session));
        Some(session)
    }

    pub(crate) async fn clear(&self) {
        let mut inner = self.inner.lock().await;
        inner.sessions.clear();
        inner.insertion_order.clear();
    }
}

#[derive(Default)]
pub(crate) struct ResponsesSessionState {
    cursor: Option<ResponsesCursor>,
}

struct ResponsesCursor {
    response_id: String,
    acknowledged_items: Vec<[u8; 32]>,
}

pub(crate) struct ResponsesRequestPlan {
    pub(crate) input: Vec<serde_json::Value>,
    pub(crate) previous_response_id: Option<String>,
}

impl ResponsesSessionState {
    pub(crate) fn plan(&mut self, full_input: &[serde_json::Value]) -> ResponsesRequestPlan {
        let fingerprints = fingerprint_items(full_input);
        if let Some(cursor) = &self.cursor
            && fingerprints.len() > cursor.acknowledged_items.len()
            && fingerprints.starts_with(&cursor.acknowledged_items)
        {
            return ResponsesRequestPlan {
                input: full_input[cursor.acknowledged_items.len()..].to_vec(),
                previous_response_id: Some(cursor.response_id.clone()),
            };
        }

        self.cursor = None;
        ResponsesRequestPlan {
            input: full_input.to_vec(),
            previous_response_id: None,
        }
    }

    pub(crate) fn commit(
        &mut self,
        full_input: &[serde_json::Value],
        response_id: Option<&str>,
        response_status: Option<&str>,
        output_items: Option<&[serde_json::Value]>,
    ) {
        let Some(response_id) = response_id.filter(|id| !id.is_empty()) else {
            self.reset();
            return;
        };
        if response_status != Some("completed") {
            self.reset();
            return;
        }
        let Some(output_items) = output_items.filter(|items| !items.is_empty()) else {
            self.reset();
            return;
        };
        let Some(replayable_output) = replayable_output_items(output_items) else {
            self.reset();
            return;
        };
        if replayable_output.is_empty() {
            self.reset();
            return;
        }

        let mut acknowledged_items = fingerprint_items(full_input);
        acknowledged_items.extend(fingerprint_items(&replayable_output));
        self.cursor = Some(ResponsesCursor {
            response_id: response_id.to_string(),
            acknowledged_items,
        });
    }

    pub(crate) fn reset(&mut self) {
        self.cursor = None;
    }
}

fn fingerprint_items(items: &[serde_json::Value]) -> Vec<[u8; 32]> {
    items
        .iter()
        .map(|item| {
            let mut canonical = item.clone();
            strip_provider_assigned_fields(&mut canonical);
            let mut hasher = Sha256::new();
            hash_json_value(&mut hasher, &canonical);
            hasher.finalize().into()
        })
        .collect()
}

fn hash_json_value(hasher: &mut Sha256, value: &serde_json::Value) {
    match value {
        serde_json::Value::Null => hasher.update([0]),
        serde_json::Value::Bool(boolean) => {
            hasher.update([1]);
            hasher.update([u8::from(*boolean)]);
        }
        serde_json::Value::Number(number) => {
            hasher.update([2]);
            hash_bytes(hasher, number.to_string().as_bytes());
        }
        serde_json::Value::String(string) => {
            hasher.update([3]);
            hash_bytes(hasher, string.as_bytes());
        }
        serde_json::Value::Array(values) => {
            hasher.update([4]);
            hasher.update((values.len() as u64).to_be_bytes());
            for value in values {
                hash_json_value(hasher, value);
            }
        }
        serde_json::Value::Object(object) => {
            hasher.update([5]);
            hasher.update((object.len() as u64).to_be_bytes());
            let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
            keys.sort_unstable();
            for key in keys {
                hash_bytes(hasher, key.as_bytes());
                if let Some(value) = object.get(key) {
                    hash_json_value(hasher, value);
                }
            }
        }
    }
}

fn hash_bytes(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

fn strip_provider_assigned_fields(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                strip_provider_assigned_fields(value);
            }
        }
        serde_json::Value::Object(object) => {
            object.remove("id");
            object.remove("status");
            for value in object.values_mut() {
                strip_provider_assigned_fields(value);
            }
        }
        _ => {}
    }
}

fn replayable_output_items(output_items: &[serde_json::Value]) -> Option<Vec<serde_json::Value>> {
    let mut replayable = Vec::new();

    for output_item in output_items {
        match output_item.get("type").and_then(|value| value.as_str())? {
            // A retained transport owns reasoning state. Durable ChatMessage
            // normalization does not reproduce it as an input item.
            "reasoning" => {}
            "message" => {
                let content = output_item.get("content")?.as_array()?;
                if content.len() != 1
                    || content[0].get("type").and_then(|value| value.as_str())
                        != Some("output_text")
                {
                    return None;
                }
                let expected = serde_json::json!({
                    "type": "message",
                    "role": "assistant",
                    "id": "provider_assigned",
                    "status": "completed",
                    "content": [{
                        "type": "output_text",
                        "text": content[0].get("text")?.as_str()?,
                        "annotations": [],
                    }],
                });
                if !same_normalized_item(output_item, &expected) {
                    return None;
                }
                replayable.push(expected);
            }
            "function_call" => {
                let expected = serde_json::json!({
                    "type": "function_call",
                    "call_id": output_item.get("call_id")?.as_str()?,
                    "name": output_item.get("name")?.as_str()?,
                    "arguments": output_item.get("arguments")?.as_str()?,
                });
                if !same_normalized_item(output_item, &expected) {
                    return None;
                }
                replayable.push(expected);
            }
            _ => return None,
        }
    }

    Some(replayable)
}

fn same_normalized_item(left: &serde_json::Value, right: &serde_json::Value) -> bool {
    fingerprint_items(std::slice::from_ref(left)) == fingerprint_items(std::slice::from_ref(right))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(text: &str) -> serde_json::Value {
        serde_json::json!({
            "role": "user",
            "content": [{"type": "input_text", "text": text}],
        })
    }

    fn assistant(text: &str) -> serde_json::Value {
        serde_json::json!({
            "type": "message",
            "role": "assistant",
            "id": "msg_local",
            "status": "completed",
            "content": [{
                "type": "output_text",
                "text": text,
                "annotations": [],
            }],
        })
    }

    fn completed_message(text: &str) -> Vec<serde_json::Value> {
        vec![serde_json::json!({
            "type": "message",
            "role": "assistant",
            "id": "msg_server",
            "status": "completed",
            "content": [{
                "type": "output_text",
                "text": text,
                "annotations": [],
            }],
        })]
    }

    struct FakeTransport;

    impl FakeTransport {
        fn complete(
            state: &mut ResponsesSessionState,
            full_input: &[serde_json::Value],
            response_id: &str,
            output: &[serde_json::Value],
        ) -> ResponsesRequestPlan {
            let plan = state.plan(full_input);
            state.commit(
                full_input,
                Some(response_id),
                Some("completed"),
                Some(output),
            );
            plan
        }
    }

    #[test]
    fn first_request_replays_full_then_follow_up_sends_only_suffix() {
        let mut state = ResponsesSessionState::default();
        let first = vec![user("first")];
        let first_plan = FakeTransport::complete(
            &mut state,
            &first,
            "resp_first",
            &completed_message("answer"),
        );
        assert_eq!(first_plan.input, first);
        assert!(first_plan.previous_response_id.is_none());

        let second = vec![user("first"), assistant("answer"), user("second")];
        let second_plan = state.plan(&second);
        assert_eq!(
            second_plan.previous_response_id.as_deref(),
            Some("resp_first")
        );
        assert_eq!(second_plan.input, vec![user("second")]);
    }

    #[test]
    fn desynchronized_transcript_replays_full() {
        let mut state = ResponsesSessionState::default();
        FakeTransport::complete(
            &mut state,
            &[user("first")],
            "resp_first",
            &completed_message("answer"),
        );

        let replacement = vec![user("authoritative replacement")];
        let plan = state.plan(&replacement);
        assert!(plan.previous_response_id.is_none());
        assert_eq!(plan.input, replacement);
    }

    #[test]
    fn incomplete_and_failed_responses_do_not_advance_cursor() {
        let mut state = ResponsesSessionState::default();
        let first = vec![user("first")];
        FakeTransport::complete(
            &mut state,
            &first,
            "resp_first",
            &completed_message("answer"),
        );
        let second = vec![user("first"), assistant("answer"), user("second")];
        assert!(state.plan(&second).previous_response_id.is_some());

        state.commit(
            &second,
            Some("resp_incomplete"),
            Some("incomplete"),
            Some(&completed_message("partial")),
        );
        assert!(state.plan(&second).previous_response_id.is_none());

        FakeTransport::complete(
            &mut state,
            &first,
            "resp_first",
            &completed_message("answer"),
        );
        assert!(state.plan(&second).previous_response_id.is_some());
        state.reset();
        assert!(state.plan(&second).previous_response_id.is_none());
    }

    #[test]
    fn structural_fingerprints_ignore_object_key_order() {
        let left: serde_json::Value =
            serde_json::from_str(r#"{"type":"message","role":"assistant"}"#).unwrap();
        let right: serde_json::Value =
            serde_json::from_str(r#"{"role":"assistant","type":"message"}"#).unwrap();
        assert_eq!(fingerprint_items(&[left]), fingerprint_items(&[right]));
    }

    #[test]
    fn reasoning_is_skipped_but_exact_function_call_is_acknowledged() {
        let output = vec![
            serde_json::json!({
                "id": "rs_server",
                "type": "reasoning",
                "encrypted_content": "opaque",
                "summary": [],
            }),
            serde_json::json!({
                "id": "fc_server",
                "type": "function_call",
                "status": "completed",
                "call_id": "call_search",
                "name": "search",
                "arguments": "{\"query\":\"rust\"}",
            }),
        ];
        let mut state = ResponsesSessionState::default();
        FakeTransport::complete(&mut state, &[user("search")], "resp_tool", &output);
        let durable_follow_up = vec![
            user("search"),
            serde_json::json!({
                "type": "function_call",
                "call_id": "call_search",
                "name": "search",
                "arguments": "{\"query\":\"rust\"}",
            }),
            serde_json::json!({
                "type": "function_call_output",
                "call_id": "call_search",
                "output": "found rust",
            }),
        ];

        let plan = state.plan(&durable_follow_up);
        assert_eq!(plan.previous_response_id.as_deref(), Some("resp_tool"));
        assert_eq!(plan.input, vec![durable_follow_up[2].clone()]);
    }

    #[test]
    fn unsupported_output_shape_disables_cursor() {
        let mut state = ResponsesSessionState::default();
        FakeTransport::complete(
            &mut state,
            &[user("first")],
            "resp_unknown",
            &[serde_json::json!({"type": "computer_call", "id": "call"})],
        );
        let full = vec![user("first"), user("second")];
        let plan = state.plan(&full);
        assert!(plan.previous_response_id.is_none());
        assert_eq!(plan.input, full);
    }

    #[tokio::test]
    async fn registry_requires_explicit_uuid_discriminator() {
        let registry = ResponsesSessionRegistry::new();
        let generic = HashMap::from([
            ("run_id".to_string(), Uuid::new_v4().to_string()),
            ("turn_id".to_string(), Uuid::new_v4().to_string()),
        ]);
        assert!(registry.session_for_metadata(&generic).await.is_none());

        let explicit =
            HashMap::from([(SESSION_METADATA_KEY.to_string(), Uuid::new_v4().to_string())]);
        assert!(registry.session_for_metadata(&explicit).await.is_some());
    }

    #[tokio::test]
    async fn registry_clear_drops_all_cursor_hints() {
        let registry = ResponsesSessionRegistry::new();
        let metadata =
            HashMap::from([(SESSION_METADATA_KEY.to_string(), Uuid::new_v4().to_string())]);
        let session = registry.session_for_metadata(&metadata).await.unwrap();
        session.lock().await.commit(
            &[user("first")],
            Some("resp_first"),
            Some("completed"),
            Some(&completed_message("answer")),
        );
        drop(session);

        registry.clear().await;

        let replacement = registry.session_for_metadata(&metadata).await.unwrap();
        let full = vec![user("first"), assistant("answer"), user("second")];
        let plan = replacement.lock().await.plan(&full);
        assert!(plan.previous_response_id.is_none());
        assert_eq!(plan.input, full);
    }

    #[tokio::test]
    async fn saturated_registry_fails_closed_until_an_idle_entry_exists() {
        let registry = ResponsesSessionRegistry::new();
        let mut active = Vec::with_capacity(MAX_RESPONSES_SESSIONS);
        for _ in 0..MAX_RESPONSES_SESSIONS {
            let metadata =
                HashMap::from([(SESSION_METADATA_KEY.to_string(), Uuid::new_v4().to_string())]);
            active.push(registry.session_for_metadata(&metadata).await.unwrap());
        }

        let overflow =
            HashMap::from([(SESSION_METADATA_KEY.to_string(), Uuid::new_v4().to_string())]);
        assert!(registry.session_for_metadata(&overflow).await.is_none());

        drop(active.pop());
        assert!(registry.session_for_metadata(&overflow).await.is_some());
    }
}
