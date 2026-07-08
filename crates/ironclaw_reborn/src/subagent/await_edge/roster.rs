//! Scope roster (§4.5): which `(tenant, user, agent, project)` scopes have
//! unclosed await-edges, discoverable without a global walk over the edge
//! tree itself (edges are scope-isolated, listed only per-scope, §4.5a).
//!
//! Deliberately diverges from this crate's other two scope-listing
//! precedents on purpose, not by oversight:
//! - `local_trigger_access/filesystem.rs`'s `ensure_index`/`query()` idiom
//!   assumes the caller already knows the scope to look up; the roster's
//!   whole job is discovering scopes the caller does *not* yet know.
//! - `goal_store.rs`/`local_trigger_access`'s nested `agents/<id>/projects/<id>`
//!   path convention breaks scope-independent enumeration at any fixed
//!   `list_dir` depth (round-3 fix, §4.5) — nested markers hide behind
//!   however many directory levels each scope's optional axes happen to use.
//!
//! The roster therefore flattens every scope's key into one percent-encoded,
//! `__`-joined filename (round-3/round-4) and shards those flat filenames
//! into 256 fixed hash-prefix directories (round-5) so boot's walk is
//! memory-bounded per shard rather than one unbounded global `list_dir`.
//! `query()`+`Page` pagination was considered and rejected (design §4.5):
//! its `OFFSET`-based cursor can skip or double-visit entries under a
//! concurrently-mutating roster, which fixed hash-prefix sharding sidesteps
//! entirely.

use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{AgentId, ProjectId, ResourceScope, ScopedPath, TenantId, UserId};
use serde::{Deserialize, Serialize};

use super::AwaitEdgeStoreError;

pub const ROSTER_SHARD_COUNT: usize = 256;

/// The structured scope a roster marker names. The marker's **payload**
/// carries this — the filename (and its shard) exists purely as the
/// enumeration key; parsing it back is a debugging convenience, never
/// load-bearing for correctness (§4.5 "Payload vs. filename").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RosterKey {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
}

impl RosterKey {
    pub fn from_resource_scope(scope: &ResourceScope) -> Self {
        Self {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
        }
    }

    pub fn to_resource_scope(&self) -> ResourceScope {
        let mut scope = ResourceScope::system();
        scope.tenant_id = self.tenant_id.clone();
        scope.user_id = self.user_id.clone();
        scope.agent_id = self.agent_id.clone();
        scope.project_id = self.project_id.clone();
        scope
    }
}

/// Percent-encode every byte outside `[A-Za-z0-9.-]` (round-4 fix — `_`
/// itself must be escaped, since `validate_scope_id` permits raw `_`/`__` in
/// ids, so a naive `__`-join is a real collision, not hypothetical).
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        let is_safe = byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-';
        if is_safe {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn percent_decode(value: &str) -> Result<String, AwaitEdgeStoreError> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(byte) = hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).map_err(|error| AwaitEdgeStoreError::Backend {
        reason: format!("roster filename percent-decode failed: {error}"),
    })
}

// Tagged presence prefixes (`-some-`/bare `-none`), not a bare `agent-<value>`
// vs. `agent-none` sentinel: `AgentId`/`ProjectId`'s `validate_scope_id` does
// not reserve the literal string `"none"`, so a real `Some("none")` id used
// to encode to the exact same segment as the missing-axis sentinel below —
// `Some("none")` and `None` collided and one silently round-tripped as the
// other, colliding roster markers/lazy-recovery scope keys for that axis
// (external review finding on this PR). The `-some-`/bare-`-none` tag makes
// the two forms textually distinct before percent-encoding even touches
// them, for every possible axis value including the literal `"none"`.
fn agent_segment(agent_id: Option<&str>) -> String {
    match agent_id {
        Some(value) => format!("agent-some-{value}"),
        None => "agent-none".to_string(),
    }
}

fn project_segment(project_id: Option<&str>) -> String {
    match project_id {
        Some(value) => format!("project-some-{value}"),
        None => "project-none".to_string(),
    }
}

/// The `__`-joined, percent-encoded filename (round-3/round-4), without the
/// `.json` suffix or shard directory.
pub fn encode_roster_filename(key: &RosterKey) -> String {
    format!(
        "{}__{}__{}__{}",
        percent_encode(key.tenant_id.as_str()),
        percent_encode(key.user_id.as_str()),
        percent_encode(&agent_segment(key.agent_id.as_ref().map(|id| id.as_str()))),
        percent_encode(&project_segment(
            key.project_id.as_ref().map(|id| id.as_str())
        )),
    )
}

/// Reverse of [`encode_roster_filename`]. Splits on `__` into exactly 4
/// parts — safe because `__` never occurs *inside* an escaped component,
/// only *between* them (round-4).
pub fn decode_roster_filename(filename: &str) -> Result<RosterKey, AwaitEdgeStoreError> {
    let parts: Vec<&str> = filename.split("__").collect();
    let [tenant, user, agent, project] = parts.as_slice() else {
        return Err(AwaitEdgeStoreError::Backend {
            reason: format!("roster filename has {} segments, expected 4", parts.len()),
        });
    };
    let tenant_id =
        TenantId::new(percent_decode(tenant)?).map_err(|error| AwaitEdgeStoreError::Backend {
            reason: format!("roster filename tenant_id invalid: {error}"),
        })?;
    let user_id =
        UserId::new(percent_decode(user)?).map_err(|error| AwaitEdgeStoreError::Backend {
            reason: format!("roster filename user_id invalid: {error}"),
        })?;
    let agent_decoded = percent_decode(agent)?;
    let agent_id = if agent_decoded == "agent-none" {
        None
    } else {
        let value = agent_decoded.strip_prefix("agent-some-").ok_or_else(|| {
            AwaitEdgeStoreError::Backend {
                reason: format!("roster filename agent segment malformed: {agent_decoded}"),
            }
        })?;
        Some(
            AgentId::new(value).map_err(|error| AwaitEdgeStoreError::Backend {
                reason: format!("roster filename agent_id invalid: {error}"),
            })?,
        )
    };
    let project_decoded = percent_decode(project)?;
    let project_id = if project_decoded == "project-none" {
        None
    } else {
        let value = project_decoded
            .strip_prefix("project-some-")
            .ok_or_else(|| AwaitEdgeStoreError::Backend {
                reason: format!("roster filename project segment malformed: {project_decoded}"),
            })?;
        Some(
            ProjectId::new(value).map_err(|error| AwaitEdgeStoreError::Backend {
                reason: format!("roster filename project_id invalid: {error}"),
            })?,
        )
    };
    Ok(RosterKey {
        tenant_id,
        user_id,
        agent_id,
        project_id,
    })
}

/// 2-lowercase-hex-digit shard prefix — the first byte of
/// `blake3::hash(filename)` (round-5). A pure function of the encoded
/// filename: deterministic, and independent of sharding-unrelated collisions
/// (two names already distinct before hashing cannot collide after it).
pub fn shard_prefix(filename: &str) -> String {
    let hash = blake3::hash(filename.as_bytes());
    format!("{:02x}", hash.as_bytes()[0])
}

pub fn roster_path(key: &RosterKey) -> Result<ScopedPath, AwaitEdgeStoreError> {
    let filename = encode_roster_filename(key);
    let shard = shard_prefix(&filename);
    ScopedPath::new(format!(
        "/turns/subagent-await-scopes/{shard}/{filename}.json"
    ))
    .map_err(|error| AwaitEdgeStoreError::Backend {
        reason: format!("invalid roster path: {error}"),
    })
}

fn roster_shard_dir(shard: &str) -> Result<ScopedPath, AwaitEdgeStoreError> {
    ScopedPath::new(format!("/turns/subagent-await-scopes/{shard}")).map_err(|error| {
        AwaitEdgeStoreError::Backend {
            reason: format!("invalid roster shard directory: {error}"),
        }
    })
}

/// Version-bumping upsert (round-5 consistency fix): present -> re-put with
/// a fresh version (a "touch", same payload); absent -> create fresh. Used
/// both as the pre-edge upsert (write-before-first-edge, §4.5) and as the
/// round-5 self-heal touch after writing the scope's first edge — same
/// function, two call sites, both unconditional and version-bumping so a
/// concurrent roster-prune's captured version is always provably stale by
/// the time either write lands.
pub async fn touch_roster_marker<F>(
    fs: &ScopedFilesystem<F>,
    key: &RosterKey,
) -> Result<(), AwaitEdgeStoreError>
where
    F: RootFilesystem + ?Sized,
{
    let system_scope = ResourceScope::system();
    let path = roster_path(key)?;
    let body = serde_json::to_vec(key).map_err(|error| AwaitEdgeStoreError::Backend {
        reason: format!("roster marker serialize failed: {error}"),
    })?;
    let existing = fs.get(&system_scope, &path).await.map_err(backend_error)?;
    let cas = match existing {
        Some(entry) => CasExpectation::Version(entry.version),
        None => CasExpectation::Absent,
    };
    match fs
        .put(
            &system_scope,
            &path,
            Entry {
                body,
                content_type: ContentType::json(),
                kind: None,
                indexed: Default::default(),
            },
            cas,
        )
        .await
    {
        Ok(_) => Ok(()),
        // A concurrent touch already landed — the marker exists either way,
        // which is all this call promises. Retry once against the fresh
        // version rather than surfacing a spurious failure.
        Err(FilesystemError::VersionMismatch { .. }) => {
            let existing = fs.get(&system_scope, &path).await.map_err(backend_error)?;
            let cas = match existing {
                Some(entry) => CasExpectation::Version(entry.version),
                None => CasExpectation::Absent,
            };
            let body = serde_json::to_vec(key).map_err(|error| AwaitEdgeStoreError::Backend {
                reason: format!("roster marker serialize failed: {error}"),
            })?;
            fs.put(
                &system_scope,
                &path,
                Entry {
                    body,
                    content_type: ContentType::json(),
                    kind: None,
                    indexed: Default::default(),
                },
                cas,
            )
            .await
            .map(|_| ())
            .or_else(|error| match error {
                // Another concurrent touch also landed in between — the
                // marker is present regardless of whose write "won"; benign.
                FilesystemError::VersionMismatch { .. } => Ok(()),
                other => Err(backend_error(other)),
            })
        }
        Err(other) => Err(backend_error(other)),
    }
}

/// CAS'd prune with the round-4/round-7 postcondition recheck (check (ii)):
/// read the marker's current version, `delete_if_version`, then re-list the
/// scope's open-edge dir; if non-empty, restore the marker via
/// [`touch_roster_marker`]. Caller-agnostic — used by both boot's recovery
/// pass and the close path's opportunistic prune (§4.5 round-7).
pub async fn prune_roster_marker<F>(
    fs: &ScopedFilesystem<F>,
    key: &RosterKey,
) -> Result<(), AwaitEdgeStoreError>
where
    F: RootFilesystem + ?Sized,
{
    let system_scope = ResourceScope::system();
    let path = roster_path(key)?;
    let Some(entry) = fs.get(&system_scope, &path).await.map_err(backend_error)? else {
        return Ok(());
    };
    match fs
        .delete_if_version(&system_scope, &path, entry.version)
        .await
    {
        Ok(())
        | Err(FilesystemError::NotFound { .. })
        | Err(FilesystemError::VersionMismatch { .. }) => {
            // NotFound: already gone (someone else pruned it) — benign.
            // VersionMismatch: a concurrent spawn's version-bumping upsert
            // landed first — the roster entry survives untouched, which is
            // exactly the (4a) convergence the design's residual-race
            // ruling names. Either way, no restore needed on this branch.
        }
        Err(other) => return Err(backend_error(other)),
    }
    // Postcondition recheck (check (ii)): does this scope's edge tree still
    // have entries? If so, restore — covers ABA across arbitrarily many
    // delete-recreate cycles for the same scope key, per §4.0's ABA-invariant
    // generalization.
    let scope = key.to_resource_scope();
    let root = super::edge_scope_root(
        key.agent_id.as_ref().map(|id| id.as_str()),
        key.project_id.as_ref().map(|id| id.as_str()),
    )?;
    let has_entries = match fs.list_dir_bounded(&scope, &root, 1).await {
        Ok(entries) => !entries.is_empty(),
        Err(FilesystemError::NotFound { .. }) => false,
        Err(other) => return Err(backend_error(other)),
    };
    if has_entries {
        touch_roster_marker(fs, key).await?;
    }
    Ok(())
}

/// Sequential walk over all 256 shard directories (§4.5 round-5) — never a
/// single global `list_dir`. Each shard's entries are decoded and yielded
/// before the next shard is listed, so peak memory is bounded by the
/// largest single shard's scope count. Undecodable filenames are skipped
/// with a `debug!` log rather than failing the whole walk.
pub async fn walk_roster_shards<F>(fs: &ScopedFilesystem<F>) -> Vec<RosterKey>
where
    F: RootFilesystem + ?Sized,
{
    let system_scope = ResourceScope::system();
    let mut keys = Vec::new();
    for shard_index in 0..ROSTER_SHARD_COUNT {
        let shard = format!("{shard_index:02x}");
        let Ok(dir) = roster_shard_dir(&shard) else {
            continue;
        };
        let entries = match fs.list_dir(&system_scope, &dir).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => continue,
            Err(error) => {
                tracing::debug!(shard = %shard, error = %error, "roster shard walk failed to list a shard, skipping");
                continue;
            }
        };
        for entry in entries {
            let Some(filename) = entry.name.strip_suffix(".json") else {
                continue;
            };
            match decode_roster_filename(filename) {
                Ok(key) => keys.push(key),
                Err(error) => {
                    tracing::debug!(filename = %filename, error = %error, "roster walk skipped undecodable filename");
                }
            }
        }
    }
    keys
}

fn backend_error(error: FilesystemError) -> AwaitEdgeStoreError {
    AwaitEdgeStoreError::Backend {
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

    use super::*;

    fn scoped_fs() -> Arc<ScopedFilesystem<InMemoryBackend>> {
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/turns").unwrap(),
            VirtualPath::new("/turns").unwrap(),
            MountPermissions::read_write_list_delete(),
        )])
        .unwrap();
        Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::new(InMemoryBackend::new()),
            mounts,
        ))
    }

    fn key(tenant: &str, user: &str, agent: Option<&str>, project: Option<&str>) -> RosterKey {
        RosterKey {
            tenant_id: TenantId::new(tenant).unwrap(),
            user_id: UserId::new(user).unwrap(),
            agent_id: agent.map(|a| AgentId::new(a).unwrap()),
            project_id: project.map(|p| ProjectId::new(p).unwrap()),
        }
    }

    // Required test (§4.5 round-4): `roster_key_encoding_disambiguates_naive_join_collision`.
    #[test]
    fn roster_key_encoding_disambiguates_naive_join_collision() {
        let a = key("a__b", "c", None, None);
        let b = key("a", "b__c", None, None);
        let encoded_a = encode_roster_filename(&a);
        let encoded_b = encode_roster_filename(&b);
        assert_ne!(encoded_a, encoded_b);
        assert_eq!(decode_roster_filename(&encoded_a).unwrap(), a);
        assert_eq!(decode_roster_filename(&encoded_b).unwrap(), b);
    }

    // External review finding on this PR: `AgentId`/`ProjectId` do not
    // reserve the literal `"none"`, so `Some("none")` must not encode (and
    // round-trip) identically to the missing-axis sentinel `None` -- a real
    // scope with agent/project id literally `"none"` must stay distinct from
    // an agentless/projectless scope, both in the encoded filename and after
    // decoding it back.
    #[test]
    fn roster_key_encoding_disambiguates_literal_none_id_from_missing_axis() {
        let with_literal_none_agent = key("tenant", "user", Some("none"), None);
        let without_agent = key("tenant", "user", None, None);
        let encoded_literal = encode_roster_filename(&with_literal_none_agent);
        let encoded_absent = encode_roster_filename(&without_agent);
        assert_ne!(
            encoded_literal, encoded_absent,
            "Some(\"none\") and None must not collide in the encoded filename"
        );
        assert_eq!(
            decode_roster_filename(&encoded_literal).unwrap(),
            with_literal_none_agent,
            "Some(\"none\") must round-trip as Some(\"none\"), not collapse to None"
        );
        assert_eq!(
            decode_roster_filename(&encoded_absent).unwrap(),
            without_agent
        );

        // Same collision class on the project axis.
        let with_literal_none_project = key("tenant", "user", None, Some("none"));
        let without_project = key("tenant", "user", None, None);
        let encoded_literal_project = encode_roster_filename(&with_literal_none_project);
        let encoded_absent_project = encode_roster_filename(&without_project);
        assert_ne!(encoded_literal_project, encoded_absent_project);
        assert_eq!(
            decode_roster_filename(&encoded_literal_project).unwrap(),
            with_literal_none_project
        );
    }

    // Required test (§4.5 round-5): `roster_shard_prefix_is_deterministic_and_well_distributed`.
    #[test]
    fn roster_shard_prefix_is_deterministic_and_well_distributed() {
        let mut buckets = std::collections::HashMap::new();
        for i in 0..4000 {
            let filename = format!("tenant-{i}__user__agent-none__project-none");
            let prefix_a = shard_prefix(&filename);
            let prefix_b = shard_prefix(&filename);
            assert_eq!(prefix_a, prefix_b, "shard prefix must be deterministic");
            *buckets.entry(prefix_a).or_insert(0u32) += 1;
        }
        let max_bucket = buckets.values().copied().max().unwrap_or(0);
        // Smoke bound, not a statistical proof: with 4000 keys over 256
        // buckets (~15.6/bucket expected), no bucket should own more than
        // ~10x the expected share.
        assert!(
            max_bucket < 300,
            "shard distribution looks pathologically skewed: {max_bucket} in one bucket"
        );
    }

    #[tokio::test]
    async fn touch_then_prune_roster_marker_round_trips() {
        let fs = scoped_fs();
        let k = key("tenant", "user", Some("agent-1"), None);
        touch_roster_marker(&fs, &k).await.unwrap();
        let path = roster_path(&k).unwrap();
        assert!(
            fs.get(&ResourceScope::system(), &path)
                .await
                .unwrap()
                .is_some()
        );
        let walked = walk_roster_shards(&fs).await;
        assert_eq!(walked, vec![k.clone()]);
    }

    /// §4.0 check (ii) / the ABA-immunity argument: `prune_roster_marker`
    /// must restore the marker if the scope's edge tree still has entries
    /// after the marker delete lands — otherwise a scope with a live edge
    /// could vanish from the roster and never get boot-recovered. Unlike
    /// `touch_then_prune_roster_marker_round_trips` above (which never calls
    /// `prune_roster_marker` at all), this seeds a real edge file under the
    /// scope's tree before pruning.
    #[tokio::test]
    async fn prune_roster_marker_restores_when_edge_dir_still_has_entries() {
        let fs = scoped_fs();
        let k = key("tenant", "user", Some("agent-1"), None);
        touch_roster_marker(&fs, &k).await.unwrap();

        // Seed one live edge file under this scope's tree so the
        // postcondition recheck sees a non-empty directory.
        let edge_path = super::super::edge_path(
            k.agent_id.as_ref().map(|id| id.as_str()),
            k.project_id.as_ref().map(|id| id.as_str()),
            ironclaw_turns::TurnRunId::new(),
            ironclaw_turns::TurnRunId::new(),
        )
        .unwrap();
        fs.put(
            &k.to_resource_scope(),
            &edge_path,
            Entry {
                body: b"{}".to_vec(),
                content_type: ContentType::json(),
                kind: None,
                indexed: Default::default(),
            },
            CasExpectation::Absent,
        )
        .await
        .unwrap();

        prune_roster_marker(&fs, &k).await.unwrap();

        let path = roster_path(&k).unwrap();
        assert!(
            fs.get(&ResourceScope::system(), &path)
                .await
                .unwrap()
                .is_some(),
            "marker must be restored — the scope still has a live edge on disk"
        );
    }
}
