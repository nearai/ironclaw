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

use std::sync::Arc;

use chrono_tz::Tz;
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{CorrelationId, InvocationId, ResourceScope};
use ironclaw_memory::{MemoryInvocation, MemoryService};
use ironclaw_memory_native::NativeMemoryService;
use ironclaw_turns::run_profile::{Locale, LoopRunContext, UserProfileContext};
use serde::Deserialize;

/// Hard cap on the profile document size. profile_set writes are small and
/// bounded; a document larger than this can only come from an external/manual
/// edit, so we refuse to spend per-turn CPU/heap parsing it and degrade to
/// no-profile instead.
const MAX_PROFILE_DOCUMENT_BYTES: usize = 64 * 1024;

/// Reads the run owner's profile document and resolves it into a validated
/// `UserProfileContext`.
///
/// Reads flow through the provider-neutral [`MemoryService::profile_read`] — the
/// same facade `profile_set` writes through — so the scope/path decision lives in
/// exactly one place (the provider) and a future provider can swap profile reads
/// alongside the rest of the memory facade. This source owns the
/// `ironclaw_memory` / `ironclaw_memory_native` dependency so the loop driver and
/// `ironclaw_reborn` never import it.
pub struct MemoryBackedUserProfileSource {
    memory_service: Arc<dyn MemoryService>,
}

impl MemoryBackedUserProfileSource {
    /// Build a source over an explicit memory provider. Production wires the
    /// native provider via [`from_filesystem`](Self::from_filesystem); tests
    /// inject a stub.
    pub fn new(memory_service: Arc<dyn MemoryService>) -> Self {
        Self { memory_service }
    }

    /// Build a source backed by the native memory provider over `filesystem`.
    /// The host owns the provider choice here (matching the memory capability),
    /// while reads still flow through the provider-neutral
    /// [`MemoryService::profile_read`].
    pub fn from_filesystem(filesystem: Arc<dyn RootFilesystem>) -> Self {
        Self::new(Arc::new(NativeMemoryService::from_filesystem(
            filesystem, None,
        )))
    }

    /// Core resolution logic. Called by `HostUserProfileSource::resolve_user_profile`
    /// implemented by the composition layer, which avoids a circular crate dependency.
    pub async fn resolve_user_profile(
        &self,
        run_context: &LoopRunContext,
    ) -> Option<UserProfileContext> {
        // Profile is keyed to the human user; the provider narrows to
        // `agent=None, project=None` internally (spec §10) regardless of the run's
        // agent/project scope, so the scope-narrowing decision is not duplicated
        // here.
        let actor = run_context.actor.as_ref()?;
        let scope = &run_context.scope;
        let invocation = MemoryInvocation {
            scope: ResourceScope {
                tenant_id: scope.tenant_id.clone(),
                user_id: actor.user_id.clone(),
                agent_id: scope.agent_id.clone(),
                project_id: scope.project_id.clone(),
                mission_id: None,
                thread_id: Some(scope.thread_id.clone()),
                invocation_id: InvocationId::new(),
            },
            correlation_id: CorrelationId::new(),
        };

        let document = match self.memory_service.profile_read(invocation).await {
            Ok(response) => response.document,
            Err(error) => {
                // silent-ok: profile is optional loop-start context; an unreadable
                // profile (incl. scope-construction failure) degrades to no-profile
                // rather than failing the user's turn. `MemoryServiceError`'s
                // `Display` is sanitized, so no backend detail leaks.
                tracing::debug!(%error, "user profile read failed; continuing without profile");
                return None;
            }
        };

        let bytes = match document {
            Some(bytes) => bytes,
            None => return None,
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

    use async_trait::async_trait;
    use ironclaw_host_api::{TenantId, ThreadId, UserId};
    use ironclaw_memory::{
        MemoryInvocation, MemoryService, MemoryServiceError, MemoryServiceProfileReadResponse,
    };
    use ironclaw_turns::{
        RunProfileResolver, TurnActor, TurnId, TurnRunId, TurnScope,
        run_profile::{InMemoryRunProfileResolver, LoopRunContext, RunProfileResolutionRequest},
    };

    use super::*;

    /// Stub memory provider: `profile_read` returns a fixed document so the tests
    /// exercise the host's parse/size-cap/validation logic, not the provider's
    /// scope/path resolution (covered by the native facade + round-trip tests).
    struct StubProfileMemoryService {
        document: Option<Vec<u8>>,
    }

    #[async_trait]
    impl MemoryService for StubProfileMemoryService {
        async fn profile_read(
            &self,
            _invocation: MemoryInvocation,
        ) -> Result<MemoryServiceProfileReadResponse, MemoryServiceError> {
            Ok(MemoryServiceProfileReadResponse {
                document: self.document.clone(),
            })
        }
    }

    fn source_with_document(document: Option<Vec<u8>>) -> MemoryBackedUserProfileSource {
        MemoryBackedUserProfileSource::new(Arc::new(StubProfileMemoryService { document }))
    }

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

    #[tokio::test]
    async fn resolves_timezone_locale_location_from_profile_doc() {
        let source = source_with_document(Some(
            r#"{"timezone":"Asia/Tokyo","locale":"ja-JP","location":"Tokyo, Japan"}"#
                .as_bytes()
                .to_vec(),
        ));
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
        let source = source_with_document(Some(
            r#"{"timezone":"Pacific Time","locale":"en-US"}"#.as_bytes().to_vec(),
        ));
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
        let source = source_with_document(None);
        let run_ctx = run_context_with_user("tenant-a", "user-1").await;
        assert!(
            source.resolve_user_profile(&run_ctx).await.is_none(),
            "missing doc must resolve to None"
        );
    }

    #[tokio::test]
    async fn no_actor_resolves_to_none() {
        let source = source_with_document(Some(r#"{"timezone":"Asia/Tokyo"}"#.as_bytes().to_vec()));
        // Build a run context without an actor.
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
        // No actor → user_id is None → should return None (no provider call needed).
        assert!(
            source.resolve_user_profile(&run_ctx).await.is_none(),
            "run context without actor must resolve to None"
        );
    }

    #[tokio::test]
    async fn all_blank_fields_resolve_to_none() {
        // A profile document with only invalid/blank fields must resolve to None
        // (the `profile == UserProfileContext::default()` guard should fire).
        let source = source_with_document(Some(
            r#"{"timezone":"Not/AZone","locale":"","location":"   "}"#
                .as_bytes()
                .to_vec(),
        ));
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
        let large_location = "A".repeat(70_000);
        let json = format!(r#"{{"location":"{}"}}"#, large_location);
        let source = source_with_document(Some(json.into_bytes()));
        let run_ctx = run_context_with_user("tenant-a", "user-1").await;
        assert!(
            source.resolve_user_profile(&run_ctx).await.is_none(),
            "oversized profile document must resolve to None (size guard, not parse error)"
        );
    }
}
