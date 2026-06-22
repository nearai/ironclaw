use ironclaw_filesystem::{CasExpectation, ContentType, Entry, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{ScopedPath, ThreadId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{MessageKind, SessionThreadError, ThreadMessageId, ThreadMessageRecord, ThreadScope};

use super::{
    PutError, deserialize, put_with_cas, scoped_path, serialize_pretty, thread_root_string,
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

    pub(super) async fn write_for_message(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message: &ThreadMessageRecord,
    ) -> Result<(), SessionThreadError> {
        if message.kind == MessageKind::Assistant
            && let Some(turn_run_id) = message.turn_run_id.as_deref()
        {
            self.write(
                scope,
                &assistant_run_index_path(scope, thread_id, turn_run_id)?,
                thread_id,
                message.message_id,
            )
            .await?;
        }
        if message.kind == MessageKind::ToolResultReference
            && let (Some(turn_run_id), Some(result_ref)) = (
                message.turn_run_id.as_deref(),
                message.tool_result_ref.as_deref(),
            )
        {
            self.write(
                scope,
                &tool_result_index_path(scope, thread_id, turn_run_id, result_ref)?,
                thread_id,
                message.message_id,
            )
            .await?;
        }
        Ok(())
    }

    pub(super) async fn read_assistant_run(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        turn_run_id: &str,
    ) -> Result<Option<ThreadMessageId>, SessionThreadError> {
        let path = assistant_run_index_path(scope, thread_id, turn_run_id)?;
        self.read(scope, thread_id, &path).await
    }

    pub(super) async fn read_tool_result(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        turn_run_id: &str,
        result_ref: &str,
    ) -> Result<Option<ThreadMessageId>, SessionThreadError> {
        let path = tool_result_index_path(scope, thread_id, turn_run_id, result_ref)?;
        self.read(scope, thread_id, &path).await
    }

    async fn read(
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

    async fn write(
        &self,
        scope: &ThreadScope,
        path: &ScopedPath,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
    ) -> Result<(), SessionThreadError> {
        let record = MessageLookupIndexRecord {
            thread_id: thread_id.clone(),
            message_id,
        };
        let body = serialize_pretty(&record)?;
        let entry = Entry::bytes(body).with_content_type(ContentType::json());
        put_with_cas(
            self.filesystem,
            &scope.to_resource_scope(),
            path,
            entry,
            CasExpectation::Any,
        )
        .await
        .map_err(|error| match error {
            PutError::VersionMismatch => SessionThreadError::Backend(format!(
                "filesystem CAS Any rejected message lookup index at {}",
                path.as_str()
            )),
            PutError::Other(error) => error,
        })
    }
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
