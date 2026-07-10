// This module is a loop-start PRODUCER (host service boundary), not a capability
// handler — that is why it lives at top-level `src/` rather than under
// `first_party_tools/` (per crate CLAUDE.md, runtime services get their own module).
//
// Note: `MemoryBackedUserProfileSource` does NOT implement `HostUserProfileSource`
// here because `ironclaw_loop_support` (which owns the trait) already depends on
// `ironclaw_host_runtime`, so a reverse dependency would be circular. The
// `impl HostUserProfileSource for MemoryBackedUserProfileSource` is added by the
// composition layer (`ironclaw_reborn_composition`) that can see both crates. This
// matches how `WorkspaceIdentityContextSource` implements `HostIdentityContextSource`:
// the struct lives in `src/workspace/` while the trait lives in `ironclaw_loop_support`.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use chrono_tz::Tz;
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{HostApiError, TenantId, UserId};
use ironclaw_memory::{MemoryContext, MemoryDocumentPath, MemoryDocumentScope};
use ironclaw_memory_native::{
    FilesystemMemoryDocumentRepository, MemoryBackend, RepositoryMemoryBackend,
};
use ironclaw_turns::run_profile::{Locale, LoopRunContext, UserProfileContext};
use serde::Deserialize;
use tokio::sync::Notify;

/// Relative path of the per-user agent-context profile document.
pub const PROFILE_DOCUMENT_PATH: &str = "context/profile.json";

/// Hard cap on the profile document size. profile_set writes are small and
/// bounded; a document larger than this can only come from an external/manual
/// edit, so we refuse to spend per-turn CPU/heap parsing it and degrade to
/// no-profile instead.
const MAX_PROFILE_DOCUMENT_BYTES: usize = 64 * 1024;
const PROFILE_CACHE_TTL: Duration = Duration::from_secs(5);
const PROFILE_CACHE_MAX_ENTRIES: usize = 1024;

/// Single home for the profile scope decision: keyed to the human user at
/// `agent=None, project=None` (spec §10) regardless of run scope. BOTH the
/// producer (read) and `profile_merge_write` (write) call this so the scope
/// narrowing — and any future project-override — lives in exactly one place.
pub(crate) fn profile_scope_and_path(
    tenant_id: &str,
    user_id: &str,
) -> Result<(MemoryDocumentScope, MemoryDocumentPath), HostApiError> {
    let scope = MemoryDocumentScope::new_with_agent(tenant_id, user_id, None, None)?;
    let path =
        MemoryDocumentPath::new_with_agent(tenant_id, user_id, None, None, PROFILE_DOCUMENT_PATH)?;
    Ok((scope, path))
}

/// Reads `context/profile.json` for the run owner and resolves it into a
/// validated `UserProfileContext`. Owns the `ironclaw_memory` dependency so the
/// loop driver and `ironclaw_reborn` never import it.
pub struct MemoryBackedUserProfileSource {
    filesystem: Arc<dyn RootFilesystem>,
    cache: Mutex<UserProfileCache>,
    inflight: Arc<Mutex<HashMap<UserProfileCacheKey, Arc<Notify>>>>,
    cache_ttl: Duration,
}

impl MemoryBackedUserProfileSource {
    pub fn new(filesystem: Arc<dyn RootFilesystem>) -> Self {
        Self {
            filesystem,
            cache: Mutex::new(UserProfileCache::default()),
            inflight: Arc::new(Mutex::new(HashMap::new())),
            cache_ttl: PROFILE_CACHE_TTL,
        }
    }

    #[cfg(test)]
    fn new_with_cache_ttl(filesystem: Arc<dyn RootFilesystem>, cache_ttl: Duration) -> Self {
        Self {
            filesystem,
            cache: Mutex::new(UserProfileCache::default()),
            inflight: Arc::new(Mutex::new(HashMap::new())),
            cache_ttl,
        }
    }

    /// Core resolution logic. Called by `HostUserProfileSource::resolve_user_profile`
    /// implemented by the composition layer, which avoids a circular crate dependency.
    pub async fn resolve_user_profile(
        &self,
        run_context: &LoopRunContext,
    ) -> Option<UserProfileContext> {
        // Profile is keyed to the human user at agent=None, project=None
        // (spec §10) regardless of the run's agent/project scope.
        let scope = &run_context.scope;
        let user_id = run_context.actor.as_ref().map(|a| a.user_id.clone())?;
        let cache_key = UserProfileCacheKey {
            tenant_id: scope.tenant_id.clone(),
            user_id,
        };
        loop {
            if let Some(profile) = self.cached_profile(&cache_key) {
                return profile;
            }
            match self.begin_profile_load(&cache_key) {
                ProfileLoad::Follower(notify) => {
                    notify.notified().await;
                }
                ProfileLoad::Leader(leader) => {
                    let profile = self
                        .resolve_user_profile_uncached(
                            cache_key.tenant_id.as_str(),
                            cache_key.user_id.as_str(),
                        )
                        .await;
                    self.store_cached_profile(cache_key.clone(), profile.clone());
                    leader.finish();
                    return profile;
                }
            }
        }
    }

    async fn resolve_user_profile_uncached(
        &self,
        tenant_id: &str,
        user_id: &str,
    ) -> Option<UserProfileContext> {
        // Shared scope helper — same keying as the writer (no duplicated decision).
        let (doc_scope, path) = match profile_scope_and_path(tenant_id, user_id) {
            Ok(pair) => pair,
            Err(error) => {
                // silent-ok: profile is optional loop-start context; a scope-construction
                // failure degrades to no-profile rather than failing the user's turn.
                tracing::debug!(%error, "user profile scope construction failed; continuing without profile");
                return None;
            }
        };
        let context = MemoryContext::new(doc_scope);

        let repository = Arc::new(FilesystemMemoryDocumentRepository::new(Arc::clone(
            &self.filesystem,
        )));
        let backend = RepositoryMemoryBackend::new(repository);

        let bytes = match backend.read_document(&context, &path).await {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return None,
            Err(error) => {
                tracing::debug!(error = %error, "user profile read failed; continuing without profile");
                // silent-ok: optional loop-start context; an unreadable profile degrades to no-profile, not a failed turn.
                return None;
            }
        };

        if bytes.len() > MAX_PROFILE_DOCUMENT_BYTES {
            // silent-ok: optional loop-start context; an oversized profile doc degrades
            // to no-profile rather than burning per-turn CPU/heap, and never fails the turn.
            tracing::debug!(
                bytes = bytes.len(),
                cap = MAX_PROFILE_DOCUMENT_BYTES,
                "user profile document exceeds size cap; continuing without profile"
            );
            return None;
        }

        let parsed: ProfileJson = match serde_json::from_slice(&bytes) {
            Ok(parsed) => parsed,
            Err(error) => {
                tracing::debug!(error = %error, "user profile JSON parse failed; continuing without profile");
                // silent-ok: optional loop-start context; a corrupt profile doc degrades to no-profile, not a failed turn.
                return None;
            }
        };

        // Never guess: invalid IANA name → None. Timezone lives in the profile.
        let timezone = parsed
            .timezone
            .as_deref()
            .and_then(|name| name.trim().parse::<Tz>().ok());
        let profile = UserProfileContext {
            timezone,
            // validated newtype; invalid → None, with a debug trail per types.md
            locale: parsed.locale.and_then(|s| match Locale::new(s) {
                Ok(l) => Some(l),
                Err(error) => {
                    tracing::debug!(%error, "locale in profile rejected; dropping field");
                    None
                }
            }),
            location: parsed.location.filter(|s| !s.trim().is_empty()),
        };

        if profile == UserProfileContext::default() {
            return None;
        }
        Some(profile)
    }

    fn cached_profile(&self, key: &UserProfileCacheKey) -> Option<Option<UserProfileContext>> {
        self.cache.lock().ok()?.get(key)
    }

    fn store_cached_profile(&self, key: UserProfileCacheKey, profile: Option<UserProfileContext>) {
        if self.cache_ttl.is_zero() || profile.is_none() {
            return;
        }
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(key, profile, self.cache_ttl);
        }
    }

    fn begin_profile_load(&self, key: &UserProfileCacheKey) -> ProfileLoad {
        let mut inflight = self
            .inflight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(notify) = inflight.get(key) {
            return ProfileLoad::Follower(Arc::clone(notify));
        }
        let notify = Arc::new(Notify::new());
        inflight.insert(key.clone(), Arc::clone(&notify));
        ProfileLoad::Leader(ProfileLoadLeader {
            inflight: Arc::clone(&self.inflight),
            key: key.clone(),
            notify,
            finished: false,
        })
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct UserProfileCacheKey {
    tenant_id: TenantId,
    user_id: UserId,
}

enum ProfileLoad {
    Leader(ProfileLoadLeader),
    Follower(Arc<Notify>),
}

struct ProfileLoadLeader {
    inflight: Arc<Mutex<HashMap<UserProfileCacheKey, Arc<Notify>>>>,
    key: UserProfileCacheKey,
    notify: Arc<Notify>,
    finished: bool,
}

impl ProfileLoadLeader {
    fn finish(mut self) {
        self.cleanup();
        self.finished = true;
    }

    fn cleanup(&self) {
        self.inflight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.key);
        self.notify.notify_waiters();
    }
}

impl Drop for ProfileLoadLeader {
    fn drop(&mut self) {
        if !self.finished {
            self.cleanup();
        }
    }
}

struct UserProfileCacheEntry {
    profile: Option<UserProfileContext>,
    expires_at: Instant,
}

#[derive(Default)]
struct UserProfileCache {
    entries: HashMap<UserProfileCacheKey, UserProfileCacheEntry>,
}

impl UserProfileCache {
    fn get(&mut self, key: &UserProfileCacheKey) -> Option<Option<UserProfileContext>> {
        let now = Instant::now();
        if let Some(entry) = self.entries.get(key)
            && entry.expires_at > now
        {
            return Some(entry.profile.clone());
        }
        self.entries.remove(key);
        None
    }

    fn insert(
        &mut self,
        key: UserProfileCacheKey,
        profile: Option<UserProfileContext>,
        ttl: Duration,
    ) {
        let now = Instant::now();
        if self.entries.len() >= PROFILE_CACHE_MAX_ENTRIES {
            self.entries.retain(|_, entry| entry.expires_at > now);
        }
        if self.entries.len() >= PROFILE_CACHE_MAX_ENTRIES
            && let Some(oldest_key) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.expires_at)
                .map(|(key, _)| key.clone())
        {
            self.entries.remove(&oldest_key);
        }
        self.entries.insert(
            key,
            UserProfileCacheEntry {
                profile,
                expires_at: now + ttl,
            },
        );
    }
}

#[derive(Debug, Deserialize, Default)]
struct ProfileJson {
    #[serde(default)]
    timezone: Option<String>,
    #[serde(default)]
    locale: Option<String>,
    #[serde(default)]
    location: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironclaw_filesystem::{InMemoryBackend, RootFilesystem};
    use ironclaw_host_api::{TenantId, ThreadId, UserId};
    use ironclaw_memory::MemoryContext;
    use ironclaw_memory_native::{
        FilesystemMemoryDocumentRepository, MemoryBackend, MemoryBackendCapabilities,
        MemoryBackendWriteOptions, RepositoryMemoryBackend,
    };
    use ironclaw_turns::{
        RunProfileResolver, TurnActor, TurnId, TurnRunId, TurnScope,
        run_profile::{InMemoryRunProfileResolver, LoopRunContext, RunProfileResolutionRequest},
    };

    use super::*;

    /// Build a test `LoopRunContext` with an actor, mirroring the `identity_context.rs` pattern.
    async fn run_context_with_user(tenant_id: &str, user_id: &str) -> LoopRunContext {
        let resolved_run_profile = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .unwrap();
        let scope = TurnScope::new(
            TenantId::new(tenant_id).unwrap(),
            None,
            None,
            ThreadId::new("thread-profile-source-test").unwrap(),
        );
        let actor = TurnActor::new(UserId::new(user_id).unwrap());
        LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved_run_profile)
            .with_actor(actor)
    }

    /// Write profile JSON bytes directly into an in-memory filesystem, scoped to user-only scope.
    async fn write_profile_json(
        fs: &Arc<dyn RootFilesystem>,
        tenant_id: &str,
        user_id: &str,
        json: &str,
    ) {
        let (scope, path) = profile_scope_and_path(tenant_id, user_id).unwrap();
        let context = MemoryContext::new(scope);
        let repository = Arc::new(FilesystemMemoryDocumentRepository::new(Arc::clone(fs)));
        let backend =
            RepositoryMemoryBackend::new(repository).with_capabilities(MemoryBackendCapabilities {
                file_documents: true,
                metadata: true,
                ..MemoryBackendCapabilities::default()
            });
        backend
            .write_document_with_backend_options(
                &context,
                &path,
                json.as_bytes(),
                &MemoryBackendWriteOptions::default(),
            )
            .await
            .expect("write_profile_json: write failed");
    }

    #[tokio::test]
    async fn resolves_timezone_locale_location_from_profile_doc() {
        let fs: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
        write_profile_json(
            &fs,
            "tenant-a",
            "user-1",
            r#"{"timezone":"Asia/Tokyo","locale":"ja-JP","location":"Tokyo, Japan"}"#,
        )
        .await;

        let source = MemoryBackedUserProfileSource::new(Arc::clone(&fs));
        let run_ctx = run_context_with_user("tenant-a", "user-1").await;
        let resolved = source.resolve_user_profile(&run_ctx).await.unwrap();

        assert_eq!(
            resolved.timezone.map(|tz| tz.name()),
            Some("Asia/Tokyo"),
            "timezone must resolve correctly"
        );
        assert_eq!(
            resolved.locale.as_ref().map(|l| l.as_str()),
            Some("ja-JP"),
            "locale must resolve correctly"
        );
        assert_eq!(
            resolved.location.as_deref(),
            Some("Tokyo, Japan"),
            "location must resolve correctly"
        );
    }

    #[tokio::test]
    async fn invalid_timezone_resolves_to_none_not_guess() {
        let fs: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
        write_profile_json(
            &fs,
            "tenant-a",
            "user-1",
            r#"{"timezone":"Pacific Time","locale":"en-US"}"#,
        )
        .await;

        let source = MemoryBackedUserProfileSource::new(Arc::clone(&fs));
        let run_ctx = run_context_with_user("tenant-a", "user-1").await;
        let resolved = source.resolve_user_profile(&run_ctx).await.unwrap();

        assert!(
            resolved.timezone.is_none(),
            "invalid IANA name must not be guessed: got {:?}",
            resolved.timezone
        );
        assert_eq!(
            resolved.locale.as_ref().map(|l| l.as_str()),
            Some("en-US"),
            "valid locale must still resolve when timezone is invalid"
        );
    }

    #[tokio::test]
    async fn missing_doc_resolves_to_none() {
        let fs: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
        let source = MemoryBackedUserProfileSource::new(fs);
        let run_ctx = run_context_with_user("tenant-a", "user-1").await;
        assert!(
            source.resolve_user_profile(&run_ctx).await.is_none(),
            "missing doc must resolve to None"
        );
    }

    #[tokio::test]
    async fn profile_cache_serves_hits_until_ttl_expires() {
        let fs: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
        write_profile_json(&fs, "tenant-a", "user-1", r#"{"locale":"ja-JP"}"#).await;

        let source = MemoryBackedUserProfileSource::new_with_cache_ttl(
            Arc::clone(&fs),
            Duration::from_secs(60),
        );
        let run_ctx = run_context_with_user("tenant-a", "user-1").await;
        let resolved = source.resolve_user_profile(&run_ctx).await.unwrap();
        assert_eq!(resolved.locale.as_ref().map(|l| l.as_str()), Some("ja-JP"));

        write_profile_json(&fs, "tenant-a", "user-1", r#"{"locale":"en-US"}"#).await;
        let cached = source.resolve_user_profile(&run_ctx).await.unwrap();
        assert_eq!(
            cached.locale.as_ref().map(|l| l.as_str()),
            Some("ja-JP"),
            "profile cache should serve the already-resolved value inside the TTL"
        );
    }

    #[tokio::test]
    async fn profile_cache_does_not_serve_misses_after_profile_is_created() {
        let fs: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
        let source = MemoryBackedUserProfileSource::new_with_cache_ttl(
            Arc::clone(&fs),
            Duration::from_secs(60),
        );
        let run_ctx = run_context_with_user("tenant-a", "user-1").await;
        assert!(source.resolve_user_profile(&run_ctx).await.is_none());

        write_profile_json(&fs, "tenant-a", "user-1", r#"{"locale":"en-US"}"#).await;
        let resolved = source.resolve_user_profile(&run_ctx).await.unwrap();
        assert_eq!(resolved.locale.as_ref().map(|l| l.as_str()), Some("en-US"));
    }

    #[tokio::test]
    async fn profile_cache_refreshes_after_ttl_expires() {
        let fs: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
        let source = MemoryBackedUserProfileSource::new_with_cache_ttl(
            Arc::clone(&fs),
            Duration::from_millis(1),
        );
        let run_ctx = run_context_with_user("tenant-a", "user-1").await;
        assert!(source.resolve_user_profile(&run_ctx).await.is_none());

        write_profile_json(&fs, "tenant-a", "user-1", r#"{"locale":"en-US"}"#).await;
        tokio::time::sleep(Duration::from_millis(10)).await;

        let resolved = source.resolve_user_profile(&run_ctx).await.unwrap();
        assert_eq!(resolved.locale.as_ref().map(|l| l.as_str()), Some("en-US"));
    }

    #[tokio::test]
    async fn no_actor_resolves_to_none() {
        let fs: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
        write_profile_json(&fs, "tenant-a", "user-1", r#"{"timezone":"Asia/Tokyo"}"#).await;

        let source = MemoryBackedUserProfileSource::new(Arc::clone(&fs));
        // Build a run context without an actor
        let resolved_run_profile = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .unwrap();
        let scope = TurnScope::new(
            TenantId::new("tenant-a").unwrap(),
            None,
            None,
            ThreadId::new("thread-no-actor").unwrap(),
        );
        let run_ctx =
            LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved_run_profile);
        // No actor → user_id is None → should return None
        assert!(
            source.resolve_user_profile(&run_ctx).await.is_none(),
            "run context without actor must resolve to None"
        );
    }

    #[tokio::test]
    async fn all_blank_fields_resolve_to_none() {
        // A profile document with only invalid/blank fields must resolve to None
        // (the `profile == UserProfileContext::default()` guard should fire).
        let fs: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
        write_profile_json(
            &fs,
            "tenant-a",
            "user-1",
            r#"{"timezone":"Not/AZone","locale":"","location":"   "}"#,
        )
        .await;

        let source = MemoryBackedUserProfileSource::new(Arc::clone(&fs));
        let run_ctx = run_context_with_user("tenant-a", "user-1").await;
        assert!(
            source.resolve_user_profile(&run_ctx).await.is_none(),
            "all-blank/invalid profile fields must resolve to None"
        );
    }

    #[tokio::test]
    async fn oversized_profile_document_resolves_to_none() {
        // A profile document larger than MAX_PROFILE_DOCUMENT_BYTES must degrade
        // to no-profile rather than burning per-turn CPU/heap parsing it.
        // The document is valid JSON (only the size guard, not a parse error, triggers).
        let fs: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
        // Build a valid JSON object whose "location" value exceeds the 64 KiB cap.
        let large_location = "A".repeat(70_000);
        let json = format!(r#"{{"location":"{}"}}"#, large_location);
        write_profile_json(&fs, "tenant-a", "user-1", &json).await;

        let source = MemoryBackedUserProfileSource::new(Arc::clone(&fs));
        let run_ctx = run_context_with_user("tenant-a", "user-1").await;
        assert!(
            source.resolve_user_profile(&run_ctx).await.is_none(),
            "oversized profile document must resolve to None (size guard, not parse error)"
        );
    }
}
