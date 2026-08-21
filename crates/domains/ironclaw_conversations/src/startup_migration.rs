//! Lossless startup migration for release-candidate channel conversation state.
//!
//! `1.0.0-rc.1` stored channel conversation authorities at provider-specific
//! roots. The generic channel host in `1.1.0-rc.1` reads an
//! extension-keyed root instead. This module owns the persisted conversation
//! grammar and therefore owns the collision-aware merge between those roots.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::Hash;

use ironclaw_filesystem::{CasExpectation, FilesystemError, RootFilesystem};
use ironclaw_host_api::{
    ids::{AgentId, ProjectId, TenantId, ThreadId},
    path::VirtualPath,
};
use serde::{
    Deserialize, Deserializer,
    de::{MapAccess, SeqAccess, Visitor},
};
use thiserror::Error;

use crate::{
    conversation_state_store::{StoredConversationState, state_entry},
    memory::InMemoryState,
};

const MIGRATION_CAS_RETRIES: usize = 5;

/// Redacted aggregate evidence for one conversation-root migration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConversationStateMigrationReport {
    pub source_present: bool,
    pub target_present: bool,
    pub source_items: usize,
    pub inserted_items: usize,
    pub unchanged_items: usize,
    pub target_written: bool,
    /// Canonical threads referenced by the migrated authority. Composition
    /// uses these typed values for the cross-domain read-back barrier; they are
    /// deliberately omitted from the redacted persisted report.
    pub referenced_threads: Vec<ConversationThreadReference>,
}

/// One canonical thread referenced by channel conversation authority.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConversationThreadReference {
    pub tenant_id: TenantId,
    pub thread_id: ThreadId,
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
}

/// Sanitized fail-closed errors from conversation-state migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ConversationStateMigrationError {
    #[error("conversation migration paths are invalid")]
    InvalidPath,
    #[error("conversation migration source state is malformed")]
    MalformedSource,
    #[error("conversation migration target state is malformed")]
    MalformedTarget,
    #[error("conversation migration found divergent authority state")]
    Conflict,
    #[error("conversation migration source changed while migration was running")]
    SourceChanged,
    #[error("conversation migration storage operation failed")]
    Storage,
    #[error("conversation migration compare-and-swap retries were exhausted")]
    Contention,
}

/// Merge one released channel conversation root into its extension-keyed root.
///
/// Both arguments are directory roots; this function reads and writes their
/// `state.json` child. The source record is never changed or deleted, keeping
/// rollback to `1.0.0-rc.1` possible. All source and target authorities are
/// parsed and checked before the first target write. Equal collisions are
/// idempotent; divergent collisions fail closed.
pub async fn migrate_conversation_state_root<F>(
    filesystem: &F,
    source_root: &VirtualPath,
    target_root: &VirtualPath,
) -> Result<ConversationStateMigrationReport, ConversationStateMigrationError>
where
    F: RootFilesystem + ?Sized,
{
    let source_path = state_path(source_root)?;
    let target_path = state_path(target_root)?;
    if source_path == target_path {
        return Err(ConversationStateMigrationError::InvalidPath);
    }

    let Some(source_entry) = filesystem
        .get(&source_path)
        .await
        .map_err(|_| ConversationStateMigrationError::Storage)?
    else {
        return Ok(ConversationStateMigrationReport::default());
    };
    let (source_revision, source_state) =
        parse_and_validate(&source_entry.entry.body, StateSide::Source)?;
    let source_items = item_count(&source_state);
    let referenced_threads = canonical_thread_references(&source_state);

    for _ in 0..MIGRATION_CAS_RETRIES {
        let target_entry = filesystem
            .get(&target_path)
            .await
            .map_err(|_| ConversationStateMigrationError::Storage)?;
        let target_present = target_entry.is_some();
        let (target_revision, mut merged) = match &target_entry {
            Some(entry) => parse_and_validate(&entry.entry.body, StateSide::Target)?,
            None => (0, InMemoryState::default()),
        };
        let (inserted_items, unchanged_items) = merge_state(&mut merged, &source_state)?;

        if inserted_items == 0 {
            verify_source_unchanged(filesystem, &source_path, &source_entry).await?;
            return Ok(ConversationStateMigrationReport {
                source_present: true,
                target_present,
                source_items,
                inserted_items,
                unchanged_items,
                target_written: false,
                referenced_threads,
            });
        }

        let revision = if target_present {
            target_revision
                .checked_add(1)
                .ok_or(ConversationStateMigrationError::MalformedTarget)?
        } else {
            source_revision.max(1)
        };
        let stored = StoredConversationState::from_state(revision, &merged);
        let body = serde_json::to_vec_pretty(&stored)
            .map_err(|_| ConversationStateMigrationError::Storage)?;
        let entry = state_entry(body, &merged);
        let cas = target_entry
            .as_ref()
            .map_or(CasExpectation::Absent, |entry| {
                CasExpectation::Version(entry.version)
            });
        match filesystem.put(&target_path, entry, cas).await {
            Ok(_) => {
                let written = filesystem
                    .get(&target_path)
                    .await
                    .map_err(|_| ConversationStateMigrationError::Storage)?
                    .ok_or(ConversationStateMigrationError::Storage)?;
                let (_, verified) = parse_and_validate(&written.entry.body, StateSide::Target)?;
                if verified != merged {
                    return Err(ConversationStateMigrationError::Storage);
                }
                verify_source_unchanged(filesystem, &source_path, &source_entry).await?;
                return Ok(ConversationStateMigrationReport {
                    source_present: true,
                    target_present,
                    source_items,
                    inserted_items,
                    unchanged_items,
                    target_written: true,
                    referenced_threads,
                });
            }
            Err(FilesystemError::VersionMismatch { .. }) => continue,
            Err(_) => return Err(ConversationStateMigrationError::Storage),
        }
    }

    Err(ConversationStateMigrationError::Contention)
}

fn canonical_thread_references(state: &InMemoryState) -> Vec<ConversationThreadReference> {
    let mut references = state
        .threads
        .iter()
        .map(|(key, record)| ConversationThreadReference {
            tenant_id: key.tenant_id.clone(),
            thread_id: key.thread_id.clone(),
            agent_id: record.agent_id.clone(),
            project_id: record.project_id.clone(),
        })
        .collect::<Vec<_>>();
    references.sort_by(|left, right| {
        (
            left.tenant_id.as_str(),
            left.thread_id.as_str(),
            left.agent_id.as_ref().map(|value| value.as_str()),
            left.project_id.as_ref().map(|value| value.as_str()),
        )
            .cmp(&(
                right.tenant_id.as_str(),
                right.thread_id.as_str(),
                right.agent_id.as_ref().map(|value| value.as_str()),
                right.project_id.as_ref().map(|value| value.as_str()),
            ))
    });
    references.dedup();
    references
}

#[derive(Clone, Copy)]
enum StateSide {
    Source,
    Target,
}

fn malformed(side: StateSide) -> ConversationStateMigrationError {
    match side {
        StateSide::Source => ConversationStateMigrationError::MalformedSource,
        StateSide::Target => ConversationStateMigrationError::MalformedTarget,
    }
}

fn parse_and_validate(
    body: &[u8],
    side: StateSide,
) -> Result<(i64, InMemoryState), ConversationStateMigrationError> {
    let mut duplicate_check = serde_json::Deserializer::from_slice(body);
    DuplicateCheckedJson::deserialize(&mut duplicate_check).map_err(|_| malformed(side))?;
    duplicate_check.end().map_err(|_| malformed(side))?;
    let stored: StoredConversationState =
        serde_json::from_slice(body).map_err(|_| malformed(side))?;
    if stored.revision < 0 {
        return Err(malformed(side));
    }
    stored
        .validate_unique_keys()
        .map_err(|()| malformed(side))?;
    let revision = stored.revision;
    let state = stored.into_state();
    if !state_is_internally_consistent(&state) {
        return Err(malformed(side));
    }
    Ok((revision, state))
}

/// JSON object keys must remain unique before serde materializes them into a
/// map. Otherwise a duplicated authority key silently becomes last-write-wins
/// before the typed migration can validate it.
struct DuplicateCheckedJson;

impl<'de> Deserialize<'de> for DuplicateCheckedJson {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(DuplicateCheckedJsonVisitor)?;
        Ok(Self)
    }
}

struct DuplicateCheckedJsonVisitor;

impl<'de> Visitor<'de> for DuplicateCheckedJsonVisitor {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JSON without duplicate object keys")
    }

    fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_str<E>(self, _: &str) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_string<E>(self, _: String) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while sequence.next_element::<DuplicateCheckedJson>()?.is_some() {}
        Ok(())
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut keys = HashSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !keys.insert(key) {
                return Err(serde::de::Error::custom("duplicate JSON object key"));
            }
            map.next_value::<DuplicateCheckedJson>()?;
        }
        Ok(())
    }
}

fn state_is_internally_consistent(state: &InMemoryState) -> bool {
    if state
        .pairing_epochs
        .keys()
        .any(|actor_key| !state.pairings.contains_key(actor_key))
    {
        return false;
    }
    for (key, binding) in &state.bindings {
        if key.tenant_id != binding.tenant_id
            || key.adapter_kind != binding.adapter_kind
            || key.adapter_installation_id != binding.adapter_installation_id
            || key.external_conversation_identity != binding.external_conversation_identity
            || state
                .source_bindings
                .get(binding.source_binding_ref.as_str())
                != Some(binding)
            || !state.threads.contains_key(&crate::memory::ThreadKey::new(
                &binding.tenant_id,
                &binding.thread_id,
            ))
        {
            return false;
        }
        let Some(reply) = state
            .reply_targets
            .get(binding.reply_target_binding_ref.as_str())
        else {
            return false;
        };
        if reply.tenant_id != binding.tenant_id
            || reply.adapter_kind != binding.adapter_kind
            || reply.adapter_installation_id != binding.adapter_installation_id
            || reply.thread_id != binding.thread_id
            || reply.source_binding_ref != binding.source_binding_ref
            || reply.reply_target_binding_ref != binding.reply_target_binding_ref
            || reply.route_access != binding.route_access
        {
            return false;
        }
    }
    state.source_bindings.len() == state.bindings.len()
        && state.reply_targets.len() == state.bindings.len()
}

fn merge_state(
    target: &mut InMemoryState,
    source: &InMemoryState,
) -> Result<(usize, usize), ConversationStateMigrationError> {
    let mut inserted = 0;
    let mut unchanged = 0;
    merge_map(
        &mut target.pairings,
        &source.pairings,
        &mut inserted,
        &mut unchanged,
    )?;
    merge_map(
        &mut target.pairing_epochs,
        &source.pairing_epochs,
        &mut inserted,
        &mut unchanged,
    )?;
    merge_map(
        &mut target.bindings,
        &source.bindings,
        &mut inserted,
        &mut unchanged,
    )?;
    merge_map(
        &mut target.source_bindings,
        &source.source_bindings,
        &mut inserted,
        &mut unchanged,
    )?;
    merge_map(
        &mut target.reply_targets,
        &source.reply_targets,
        &mut inserted,
        &mut unchanged,
    )?;
    merge_map(
        &mut target.threads,
        &source.threads,
        &mut inserted,
        &mut unchanged,
    )?;
    merge_map(
        &mut target.external_event_routes,
        &source.external_event_routes,
        &mut inserted,
        &mut unchanged,
    )?;
    merge_map(
        &mut target.message_idempotency,
        &source.message_idempotency,
        &mut inserted,
        &mut unchanged,
    )?;
    merge_map(
        &mut target.message_replays,
        &source.message_replays,
        &mut inserted,
        &mut unchanged,
    )?;
    merge_map(
        &mut target.submission_keys,
        &source.submission_keys,
        &mut inserted,
        &mut unchanged,
    )?;
    merge_map(
        &mut target.submitted_message_responses,
        &source.submitted_message_responses,
        &mut inserted,
        &mut unchanged,
    )?;

    let mut target_messages: HashMap<_, _> = target
        .messages
        .iter()
        .map(|message| (message.accepted.message_ref.clone(), message.clone()))
        .collect();
    if target_messages.len() != target.messages.len() {
        return Err(ConversationStateMigrationError::MalformedTarget);
    }
    for message in &source.messages {
        match target_messages.get(&message.accepted.message_ref) {
            Some(existing) if existing == message => unchanged += 1,
            Some(_) => return Err(ConversationStateMigrationError::Conflict),
            None => {
                target.messages.push(message.clone());
                target_messages.insert(message.accepted.message_ref.clone(), message.clone());
                inserted += 1;
            }
        }
    }
    Ok((inserted, unchanged))
}

fn merge_map<K, V>(
    target: &mut HashMap<K, V>,
    source: &HashMap<K, V>,
    inserted: &mut usize,
    unchanged: &mut usize,
) -> Result<(), ConversationStateMigrationError>
where
    K: Clone + Eq + Hash,
    V: Clone + Eq,
{
    for (key, value) in source {
        match target.get(key) {
            Some(existing) if existing == value => *unchanged += 1,
            Some(_) => return Err(ConversationStateMigrationError::Conflict),
            None => {
                target.insert(key.clone(), value.clone());
                *inserted += 1;
            }
        }
    }
    Ok(())
}

fn item_count(state: &InMemoryState) -> usize {
    state.pairings.len()
        + state.pairing_epochs.len()
        + state.bindings.len()
        + state.source_bindings.len()
        + state.reply_targets.len()
        + state.threads.len()
        + state.external_event_routes.len()
        + state.message_idempotency.len()
        + state.message_replays.len()
        + state.submission_keys.len()
        + state.submitted_message_responses.len()
        + state.messages.len()
}

fn state_path(root: &VirtualPath) -> Result<VirtualPath, ConversationStateMigrationError> {
    VirtualPath::new(format!(
        "{}/state.json",
        root.as_str().trim_end_matches('/')
    ))
    .map_err(|_| ConversationStateMigrationError::InvalidPath)
}

async fn verify_source_unchanged<F>(
    filesystem: &F,
    source_path: &VirtualPath,
    expected: &ironclaw_filesystem::VersionedEntry,
) -> Result<(), ConversationStateMigrationError>
where
    F: RootFilesystem + ?Sized,
{
    let current = filesystem
        .get(source_path)
        .await
        .map_err(|_| ConversationStateMigrationError::Storage)?
        .ok_or(ConversationStateMigrationError::SourceChanged)?;
    if current.version != expected.version || current.entry.body != expected.entry.body {
        return Err(ConversationStateMigrationError::SourceChanged);
    }
    Ok(())
}
