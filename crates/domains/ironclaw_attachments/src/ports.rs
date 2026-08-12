//! The attachment landing/read ports.
//!
//! These are declared here, next to the landing routine they front, rather than
//! in `ironclaw_assistant` (PROPOSAL §6.4.9, "the single landing routine **plus
//! its ports**" — ending the three-crate spread where the contract, the routine
//! and the adapter each lived in a different crate). The implementations stay
//! wherever the behavior does: [`crate::ProjectScopedAttachmentLander`] is the
//! default one, over a project-scoped `ScopedFilesystem`.
//!
//! They error with [`ProductSurfaceError`] because the caller that fails is a
//! product surface — the WebUI bytes endpoint maps
//! [`ProductSurfaceErrorCode::NotFound`](ironclaw_product_contracts::surface::ProductSurfaceErrorCode)
//! straight onto its 404. Narrowing the ports onto an attachment-owned error
//! would move that HTTP status mapping, which is a behavior change and belongs
//! to its own slice; `ironclaw_product_contracts` is the *neutral* product-tier
//! contract crate and sits in the `contracts` layer, so naming it here is a
//! downward edge like `host_api`.

use async_trait::async_trait;
use ironclaw_common::AttachmentRef;
use ironclaw_host_api::attachment::InboundAttachment;
use ironclaw_product_contracts::surface::ProductSurfaceError;
use ironclaw_threads::ThreadScope;

/// What one [`InboundAttachmentLander::cleanup_stale`] pass reconciled.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AttachmentCleanupReport {
    pub scanned_batches: usize,
    pub deleted_batches: usize,
}

/// Lands inbound attachment bytes into durable, agent-accessible storage and
/// returns the transcript references to persist on the user message.
///
/// Injected by host composition, which owns the project-scoped filesystem
/// authority. `message_id` is a stable per-message id (the idempotency key)
/// used only to disambiguate the storage path; the implementation writes
/// through the same `MountView` the agent's file tools resolve through, so
/// landed bytes are readable by `file_read`/`list_dir` in later turns.
#[async_trait]
pub trait InboundAttachmentLander: Send + Sync {
    async fn land(
        &self,
        thread_scope: &ThreadScope,
        message_id: &str,
        attachments: Vec<InboundAttachment>,
    ) -> Result<Vec<AttachmentRef>, ProductSurfaceError>;

    /// Remove one complete batch previously returned by [`Self::land`].
    ///
    /// The inbound workflow calls this only when durable message acceptance
    /// fails after landing. Implementations must constrain deletion to the
    /// batch represented by `attachments`; they must never sweep unrelated
    /// workspace paths.
    async fn rollback(
        &self,
        thread_scope: &ThreadScope,
        attachments: &[AttachmentRef],
    ) -> Result<(), ProductSurfaceError>;

    /// Reconcile old committed batches against an exhaustive set of durable
    /// attachment storage keys for this exact thread scope.
    ///
    /// Callers must skip this operation when their reference scan was
    /// truncated. The complete snapshot may include attachment domains the
    /// implementation does not own, such as agent-created outbound workspace
    /// files; implementations ignore those references and fail closed when no
    /// owned reference proves the snapshot usable. Implementations keep a
    /// reconciliation window and bounded filesystem scan so recent in-flight
    /// work and unrelated workspace paths are never removed.
    async fn cleanup_stale(
        &self,
        thread_scope: &ThreadScope,
        referenced_storage_keys: &[String],
    ) -> Result<AttachmentCleanupReport, ProductSurfaceError>;
}

/// Reads a landed attachment's bytes back for the WebUI bytes endpoint. The
/// read counterpart of [`InboundAttachmentLander`]: host composition implements
/// it over the same project-scoped workspace filesystem the lander wrote
/// through, so `storage_key` is re-scoped through that mount authority and never
/// treated as a host path.
#[async_trait]
pub trait InboundAttachmentReader: Send + Sync {
    async fn read(
        &self,
        thread_scope: &ThreadScope,
        storage_key: &str,
    ) -> Result<Vec<u8>, ProductSurfaceError>;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::Mutex;

    use ironclaw_host_api::ids::{AgentId, TenantId, UserId};
    use ironclaw_product_contracts::surface::{ProductSurfaceErrorCode, ProductSurfaceErrorKind};

    use super::*;

    /// A named function, not a closure, so the mapping the double uses is the
    /// same shape production code would extract and test.
    fn not_found() -> ProductSurfaceError {
        ProductSurfaceError {
            code: ProductSurfaceErrorCode::NotFound,
            kind: ProductSurfaceErrorKind::NotFound,
            status_code: 404,
            retryable: false,
            field: None,
            validation_code: None,
        }
    }

    fn scope(agent: &str) -> ThreadScope {
        ThreadScope {
            tenant_id: TenantId::new("tenant-1").expect("tenant"),
            agent_id: AgentId::new(agent).expect("agent"),
            project_id: None,
            owner_user_id: Some(UserId::new("user-1").expect("user")),
            mission_id: None,
        }
    }

    fn attachment_ref(storage_key: &str) -> AttachmentRef {
        AttachmentRef {
            id: storage_key.to_string(),
            kind: ironclaw_common::AttachmentKind::Document,
            mime_type: "text/plain".to_string(),
            filename: Some("a.txt".to_string()),
            size_bytes: Some(1),
            storage_key: Some(storage_key.to_string()),
            extracted_text: None,
        }
    }

    /// Records every argument each port method receives, and answers from a
    /// per-`(scope, key)` table — so an implementation that ignored the scope
    /// (or the message id) could not satisfy the assertions below.
    #[derive(Default)]
    struct RecordingAttachments {
        landed: Mutex<Vec<(ThreadScope, String, usize)>>,
        rolled_back: Mutex<Vec<(ThreadScope, Vec<String>)>>,
        swept: Mutex<Vec<(ThreadScope, Vec<String>)>>,
        readable: Mutex<Vec<(ThreadScope, String, Vec<u8>)>>,
    }

    #[async_trait]
    impl InboundAttachmentLander for RecordingAttachments {
        async fn land(
            &self,
            thread_scope: &ThreadScope,
            message_id: &str,
            attachments: Vec<InboundAttachment>,
        ) -> Result<Vec<AttachmentRef>, ProductSurfaceError> {
            self.landed.lock().expect("lock").push((
                thread_scope.clone(),
                message_id.to_string(),
                attachments.len(),
            ));
            Ok(attachments
                .iter()
                .enumerate()
                .map(|(index, _)| attachment_ref(&format!("/workspace/{message_id}/{index}")))
                .collect())
        }

        async fn rollback(
            &self,
            thread_scope: &ThreadScope,
            attachments: &[AttachmentRef],
        ) -> Result<(), ProductSurfaceError> {
            self.rolled_back.lock().expect("lock").push((
                thread_scope.clone(),
                attachments
                    .iter()
                    .filter_map(|a| a.storage_key.clone())
                    .collect(),
            ));
            Ok(())
        }

        async fn cleanup_stale(
            &self,
            thread_scope: &ThreadScope,
            referenced_storage_keys: &[String],
        ) -> Result<AttachmentCleanupReport, ProductSurfaceError> {
            self.swept
                .lock()
                .expect("lock")
                .push((thread_scope.clone(), referenced_storage_keys.to_vec()));
            Ok(AttachmentCleanupReport {
                scanned_batches: referenced_storage_keys.len(),
                deleted_batches: 0,
            })
        }
    }

    #[async_trait]
    impl InboundAttachmentReader for RecordingAttachments {
        async fn read(
            &self,
            thread_scope: &ThreadScope,
            storage_key: &str,
        ) -> Result<Vec<u8>, ProductSurfaceError> {
            self.readable
                .lock()
                .expect("lock")
                .iter()
                .find(|(scope, key, _)| scope == thread_scope && key == storage_key)
                .map(|(_, _, bytes)| bytes.clone())
                .ok_or_else(not_found)
        }
    }

    /// Both ports are held as `Arc<dyn _>` by the product surface and by
    /// composition; a non-dyn-compatible signature would only fail at the far
    /// call site, so it is pinned here at the declaration.
    #[test]
    fn both_ports_are_usable_as_trait_objects() {
        let shared = Arc::new(RecordingAttachments::default());
        let _lander: Arc<dyn InboundAttachmentLander> = shared.clone();
        let _reader: Arc<dyn InboundAttachmentReader> = shared;
    }

    #[tokio::test]
    async fn the_lander_port_hands_the_implementation_its_scope_and_message_id() {
        let lander = RecordingAttachments::default();
        let scope_a = scope("agent-a");
        let refs = lander
            .land(
                &scope_a,
                "message-7",
                vec![InboundAttachment {
                    id: "att-1".to_string(),
                    mime_type: "text/plain".to_string(),
                    filename: Some("a.txt".to_string()),
                    bytes: vec![1],
                }],
            )
            .await
            .expect("land");

        let landed = lander.landed.lock().expect("lock").clone();
        assert_eq!(landed, vec![(scope_a.clone(), "message-7".to_string(), 1)]);
        assert_eq!(refs.len(), 1);
        let storage_key = refs[0]
            .storage_key
            .as_deref()
            .expect("a landed ref carries its storage key");
        assert!(
            storage_key.contains("message-7"),
            "the message id must reach the storage path: {storage_key}"
        );

        lander
            .rollback(&scope_a, &refs)
            .await
            .expect("rollback the batch just landed");
        let rolled = lander.rolled_back.lock().expect("lock").clone();
        assert_eq!(
            rolled,
            vec![(scope_a, vec![storage_key.to_string()])],
            "rollback receives exactly the batch `land` returned, scope included"
        );
    }

    #[tokio::test]
    async fn the_cleanup_port_hands_the_implementation_the_reference_snapshot() {
        let lander = RecordingAttachments::default();
        let scope_a = scope("agent-a");
        let report = lander
            .cleanup_stale(&scope_a, &["/workspace/x".to_string()])
            .await
            .expect("cleanup");
        assert_eq!(report.scanned_batches, 1);
        assert_eq!(
            lander.swept.lock().expect("lock").clone(),
            vec![(scope_a, vec!["/workspace/x".to_string()])]
        );
    }

    /// The read port carries the scope so one thread's storage key cannot serve
    /// another's bytes. Asserted in both directions against a double that keys
    /// on `(scope, key)` — a double that ignored the scope would pass a
    /// one-directional check.
    #[tokio::test]
    async fn the_read_port_answer_can_differ_by_thread_scope() {
        let reader = RecordingAttachments::default();
        let scope_a = scope("agent-a");
        let scope_b = scope("agent-b");
        reader.readable.lock().expect("lock").push((
            scope_a.clone(),
            "/workspace/k".to_string(),
            vec![7],
        ));

        assert_eq!(
            reader.read(&scope_a, "/workspace/k").await.expect("read"),
            vec![7]
        );
        let denied = reader
            .read(&scope_b, "/workspace/k")
            .await
            .expect_err("a different scope must not resolve the same key");
        assert_eq!(denied.code, ProductSurfaceErrorCode::NotFound);

        reader.readable.lock().expect("lock").push((
            scope_b.clone(),
            "/workspace/k".to_string(),
            vec![9],
        ));
        assert_eq!(
            reader.read(&scope_b, "/workspace/k").await.expect("read"),
            vec![9],
            "and the other direction: the same key under its own scope resolves"
        );
    }
}
