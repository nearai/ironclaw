use ironclaw_filesystem::{
    Entry, Filter, IndexKind, IndexSpec, IndexValue, OrderedPage, RootFilesystem, ScopedFilesystem,
    SortDirection,
};
use ironclaw_host_api::{
    ids::{InvocationId, ThreadId},
    path::ScopedPath,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    CapabilityDisplayPreviewEnvelope, MessageKind, SessionThreadError, ThreadMessageId,
    ThreadMessageRecord, ThreadScope,
};

use super::{
    deserialize, fs_index_key, fs_index_name, messages_root, scoped_path, serialize_pretty,
    thread_root_string,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MessageLookupIndexRecord {
    thread_id: ThreadId,
    message_id: ThreadMessageId,
}

pub(super) struct MessageLookupIndexStore<'a, F>
where
    F: RootFilesystem,
{
    filesystem: &'a ScopedFilesystem<F>,
}

impl<'a, F> MessageLookupIndexStore<'a, F>
where
    F: RootFilesystem,
{
    pub(super) fn new(filesystem: &'a ScopedFilesystem<F>) -> Self {
        Self { filesystem }
    }

    pub(super) async fn read_first_user(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
    ) -> Result<Option<ThreadMessageId>, SessionThreadError> {
        if let Some(message_id) = self
            .query_message_id(
                scope,
                thread_id,
                first_user_index_spec()?,
                vec![("lookup_first_user", IndexValue::Bool(true))],
                SortDirection::Ascending,
            )
            .await?
        {
            return Ok(Some(message_id));
        }
        self.read_legacy(scope, thread_id, &first_user_index_path(scope, thread_id)?)
            .await
    }

    pub(super) async fn read_capability_preview(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        turn_run_id: &str,
        invocation_id: InvocationId,
    ) -> Result<Option<ThreadMessageId>, SessionThreadError> {
        if let Some(message_id) = self
            .query_message_id(
                scope,
                thread_id,
                capability_preview_index_spec()?,
                vec![
                    (
                        "lookup_capability_run_id",
                        IndexValue::Text(turn_run_id.to_string()),
                    ),
                    (
                        "lookup_invocation_id",
                        IndexValue::Text(invocation_id.to_string()),
                    ),
                ],
                SortDirection::Descending,
            )
            .await?
        {
            return Ok(Some(message_id));
        }
        self.read_legacy(
            scope,
            thread_id,
            &capability_preview_index_path(scope, thread_id, turn_run_id, invocation_id)?,
        )
        .await
    }

    pub(super) async fn read_assistant_run(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        turn_run_id: &str,
    ) -> Result<Option<ThreadMessageId>, SessionThreadError> {
        if let Some(message_id) = self
            .query_message_id(
                scope,
                thread_id,
                assistant_run_index_spec()?,
                vec![(
                    "lookup_assistant_run_id",
                    IndexValue::Text(turn_run_id.to_string()),
                )],
                SortDirection::Descending,
            )
            .await?
        {
            return Ok(Some(message_id));
        }
        self.read_legacy(
            scope,
            thread_id,
            &assistant_run_index_path(scope, thread_id, turn_run_id)?,
        )
        .await
    }

    pub(super) async fn read_tool_result(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        turn_run_id: &str,
        result_ref: &str,
    ) -> Result<Option<ThreadMessageId>, SessionThreadError> {
        if let Some(message_id) = self
            .query_message_id(
                scope,
                thread_id,
                tool_result_index_spec()?,
                vec![
                    (
                        "lookup_tool_result_run_id",
                        IndexValue::Text(turn_run_id.to_string()),
                    ),
                    (
                        "lookup_tool_result_ref",
                        IndexValue::Text(result_ref.to_string()),
                    ),
                ],
                SortDirection::Descending,
            )
            .await?
        {
            return Ok(Some(message_id));
        }
        self.read_legacy(
            scope,
            thread_id,
            &tool_result_index_path(scope, thread_id, turn_run_id, result_ref)?,
        )
        .await
    }

    pub(super) async fn read_tool_result_provider_call(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        turn_run_id: &str,
        result_ref: &str,
        provider_call_id: &str,
    ) -> Result<Option<ThreadMessageId>, SessionThreadError> {
        if let Some(message_id) = self
            .query_message_id(
                scope,
                thread_id,
                tool_result_provider_call_index_spec()?,
                vec![
                    (
                        "lookup_tool_result_run_id",
                        IndexValue::Text(turn_run_id.to_string()),
                    ),
                    (
                        "lookup_tool_result_ref",
                        IndexValue::Text(result_ref.to_string()),
                    ),
                    (
                        "lookup_provider_call_id",
                        IndexValue::Text(provider_call_id.to_string()),
                    ),
                ],
                SortDirection::Descending,
            )
            .await?
        {
            return Ok(Some(message_id));
        }
        self.read_legacy(
            scope,
            thread_id,
            &tool_result_provider_call_index_path(
                scope,
                thread_id,
                turn_run_id,
                result_ref,
                provider_call_id,
            )?,
        )
        .await
    }

    async fn query_message_id(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        index: IndexSpec,
        lookup_filters: Vec<(&str, IndexValue)>,
        direction: SortDirection,
    ) -> Result<Option<ThreadMessageId>, SessionThreadError> {
        let mut filters = Vec::with_capacity(lookup_filters.len() + 1);
        filters.push(Filter::Eq {
            key: fs_index_key("thread_id")?,
            value: IndexValue::Text(thread_id.to_string()),
        });
        for (key, value) in lookup_filters {
            filters.push(Filter::Eq {
                key: fs_index_key(key)?,
                value,
            });
        }
        let page = OrderedPage::new(
            index.name,
            fs_index_key("sequence")?,
            fs_index_key("message_id")?,
            direction,
            1,
        );
        self.filesystem
            .query_ordered(
                &scope.to_resource_scope(),
                &messages_root(scope, thread_id)?,
                &Filter::And(filters),
                &page,
            )
            .await?
            .into_iter()
            .next()
            .map(|row| {
                deserialize::<ThreadMessageRecord>(&row.entry.body).map(|row| row.message_id)
            })
            .transpose()
    }

    async fn read_legacy(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        path: &ScopedPath,
    ) -> Result<Option<ThreadMessageId>, SessionThreadError> {
        let Some(versioned) = self
            .filesystem
            .get(&scope.to_resource_scope(), path)
            .await?
        else {
            return Ok(None);
        };
        let record = deserialize::<MessageLookupIndexRecord>(&versioned.entry.body)?;
        if &record.thread_id != thread_id {
            return Ok(None);
        }
        Ok(Some(record.message_id))
    }
}

pub(super) fn with_message_lookup_projections(
    mut entry: Entry,
    message: &ThreadMessageRecord,
) -> Result<Entry, SessionThreadError> {
    if message.kind == MessageKind::Assistant
        && let Some(turn_run_id) = message.turn_run_id.as_deref()
    {
        entry.indexed.insert(
            fs_index_key("lookup_assistant_run_id")?,
            IndexValue::Text(turn_run_id.to_string()),
        );
    }
    if message.kind == MessageKind::ToolResultReference
        && let (Some(turn_run_id), Some(result_ref)) = (
            message.turn_run_id.as_deref(),
            message.tool_result_ref.as_deref(),
        )
    {
        entry.indexed.insert(
            fs_index_key("lookup_tool_result_run_id")?,
            IndexValue::Text(turn_run_id.to_string()),
        );
        entry.indexed.insert(
            fs_index_key("lookup_tool_result_ref")?,
            IndexValue::Text(result_ref.to_string()),
        );
        if let Some(provider_call_id) = message
            .tool_result_provider_call
            .as_ref()
            .map(|provider_call| provider_call.provider_call_id.as_str())
        {
            entry.indexed.insert(
                fs_index_key("lookup_provider_call_id")?,
                IndexValue::Text(provider_call_id.to_string()),
            );
        }
    }
    if message.kind == MessageKind::User {
        entry
            .indexed
            .insert(fs_index_key("lookup_first_user")?, IndexValue::Bool(true));
    }
    if message.kind == MessageKind::CapabilityDisplayPreview
        && let (Some(turn_run_id), Some(invocation_id)) = (
            message.turn_run_id.as_deref(),
            CapabilityDisplayPreviewEnvelope::invocation_id_from_json(message.content.as_deref())
                .map_err(SessionThreadError::Serialization)?,
        )
    {
        entry.indexed.insert(
            fs_index_key("lookup_capability_run_id")?,
            IndexValue::Text(turn_run_id.to_string()),
        );
        entry.indexed.insert(
            fs_index_key("lookup_invocation_id")?,
            IndexValue::Text(invocation_id.to_string()),
        );
    }
    Ok(entry)
}

pub(super) fn lookup_index_specs() -> Result<[IndexSpec; 5], SessionThreadError> {
    Ok([
        assistant_run_index_spec()?,
        tool_result_index_spec()?,
        tool_result_provider_call_index_spec()?,
        first_user_index_spec()?,
        capability_preview_index_spec()?,
    ])
}

fn assistant_run_index_spec() -> Result<IndexSpec, SessionThreadError> {
    lookup_index_spec(
        "thread_message_assistant_run_v1",
        &["lookup_assistant_run_id"],
    )
}

fn tool_result_index_spec() -> Result<IndexSpec, SessionThreadError> {
    lookup_index_spec(
        "thread_message_tool_result_v1",
        &["lookup_tool_result_run_id", "lookup_tool_result_ref"],
    )
}

fn tool_result_provider_call_index_spec() -> Result<IndexSpec, SessionThreadError> {
    lookup_index_spec(
        "thread_message_tool_result_provider_call_v1",
        &[
            "lookup_tool_result_run_id",
            "lookup_tool_result_ref",
            "lookup_provider_call_id",
        ],
    )
}

fn first_user_index_spec() -> Result<IndexSpec, SessionThreadError> {
    lookup_index_spec("thread_message_first_user_v1", &["lookup_first_user"])
}

fn capability_preview_index_spec() -> Result<IndexSpec, SessionThreadError> {
    lookup_index_spec(
        "thread_message_capability_preview_v1",
        &["lookup_capability_run_id", "lookup_invocation_id"],
    )
}

fn lookup_index_spec(name: &str, lookup_keys: &[&str]) -> Result<IndexSpec, SessionThreadError> {
    let mut keys = Vec::with_capacity(lookup_keys.len() + 3);
    keys.push(fs_index_key("thread_id")?);
    for key in lookup_keys {
        keys.push(fs_index_key(key)?);
    }
    keys.push(fs_index_key("sequence")?);
    keys.push(fs_index_key("message_id")?);
    Ok(IndexSpec::new(fs_index_name(name)?, keys, IndexKind::Exact))
}

fn first_user_index_path(
    scope: &ThreadScope,
    thread_id: &ThreadId,
) -> Result<ScopedPath, SessionThreadError> {
    scoped_path(&format!(
        "{}/indexes/first-user.json",
        thread_root_string(scope, thread_id)
    ))
}

fn capability_preview_index_path(
    scope: &ThreadScope,
    thread_id: &ThreadId,
    turn_run_id: &str,
    invocation_id: InvocationId,
) -> Result<ScopedPath, SessionThreadError> {
    #[derive(Serialize)]
    struct CapabilityPreviewIndexKey<'a> {
        turn_run_id: &'a str,
        invocation_id: InvocationId,
    }
    let key = lookup_index_key(
        "capability-preview",
        &CapabilityPreviewIndexKey {
            turn_run_id,
            invocation_id,
        },
    )?;
    scoped_path(&format!(
        "{}/indexes/capability-previews/{key}.json",
        thread_root_string(scope, thread_id)
    ))
}

fn assistant_run_index_path(
    scope: &ThreadScope,
    thread_id: &ThreadId,
    turn_run_id: &str,
) -> Result<ScopedPath, SessionThreadError> {
    #[derive(Serialize)]
    struct AssistantRunIndexKey<'a> {
        turn_run_id: &'a str,
    }
    let key = lookup_index_key("assistant-run", &AssistantRunIndexKey { turn_run_id })?;
    scoped_path(&format!(
        "{}/indexes/assistant-runs/{key}.json",
        thread_root_string(scope, thread_id)
    ))
}

fn tool_result_index_path(
    scope: &ThreadScope,
    thread_id: &ThreadId,
    turn_run_id: &str,
    result_ref: &str,
) -> Result<ScopedPath, SessionThreadError> {
    #[derive(Serialize)]
    struct ToolResultIndexKey<'a> {
        turn_run_id: &'a str,
        result_ref: &'a str,
    }
    let key = lookup_index_key(
        "tool-result",
        &ToolResultIndexKey {
            turn_run_id,
            result_ref,
        },
    )?;
    scoped_path(&format!(
        "{}/indexes/tool-results/{key}.json",
        thread_root_string(scope, thread_id)
    ))
}

fn tool_result_provider_call_index_path(
    scope: &ThreadScope,
    thread_id: &ThreadId,
    turn_run_id: &str,
    result_ref: &str,
    provider_call_id: &str,
) -> Result<ScopedPath, SessionThreadError> {
    #[derive(Serialize)]
    struct ToolResultProviderCallIndexKey<'a> {
        turn_run_id: &'a str,
        result_ref: &'a str,
        provider_call_id: &'a str,
    }
    let key = lookup_index_key(
        "tool-result-provider-call",
        &ToolResultProviderCallIndexKey {
            turn_run_id,
            result_ref,
            provider_call_id,
        },
    )?;
    scoped_path(&format!(
        "{}/indexes/tool-results/{key}.json",
        thread_root_string(scope, thread_id)
    ))
}

fn lookup_index_key<T: Serialize>(prefix: &str, key: &T) -> Result<String, SessionThreadError> {
    let payload = serialize_pretty(key)?;
    let digest = Sha256::digest(&payload);
    let mut output = String::with_capacity(prefix.len() + 1 + digest.len() * 2);
    output.push_str(prefix);
    output.push('-');
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}")
            .map_err(|error| SessionThreadError::Serialization(error.to_string()))?;
    }
    Ok(output)
}
