//! Composition half of run-completion external presentation (2026-08-13
//! design §6.1, §7.9): the host-owned web-app enrollment probe the
//! `local_os` policy consults.
//!
//! The probe reads the SAME host-owned delivery registrations the delivery
//! coordinator resolves for actual pushes, so "Enrolled" here can never
//! diverge from what a push would use. Browser-instance correlation rides
//! inside the registration's channel-opaque `document` (interpreted only
//! where it is used, mirroring the web-app adapter's own parsing rule);
//! records that predate correlation degrade to profile-level presence.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_assistant::run_completions::push::{
    WebAppEnrollmentProbe, WebAppEnrollmentSnapshot,
};
use ironclaw_assistant::run_completions::store::RunCompletionOwner;
use ironclaw_host_api::ids::ExtensionId;
use ironclaw_product_contracts::delivery::{
    DeliveryRegistrationScope, DeliveryRegistrationService,
};

pub(crate) struct RegistrationEnrollmentProbe {
    registrations: Arc<dyn DeliveryRegistrationService>,
    extension_id: ExtensionId,
}

impl RegistrationEnrollmentProbe {
    pub(crate) fn new(
        registrations: Arc<dyn DeliveryRegistrationService>,
        extension_id: ExtensionId,
    ) -> Self {
        Self {
            registrations,
            extension_id,
        }
    }
}

#[async_trait]
impl WebAppEnrollmentProbe for RegistrationEnrollmentProbe {
    async fn enrollment(
        &self,
        owner: &RunCompletionOwner,
    ) -> Result<WebAppEnrollmentSnapshot, String> {
        let scope = DeliveryRegistrationScope {
            tenant_id: owner.tenant_id.clone(),
            user_id: owner.user_id.clone(),
            extension_id: self.extension_id.clone(),
        };
        let registrations = self
            .registrations
            .list(&scope)
            .await
            .map_err(|error| error.to_string())?;
        let mut snapshot = WebAppEnrollmentSnapshot::default();
        for registration in registrations {
            match browser_instance_of(&registration.document) {
                Some(instance_id) => snapshot.instance_ids.push(instance_id),
                None => snapshot.uncorrelated += 1,
            }
        }
        Ok(snapshot)
    }
}

/// The optional `browser_instance_id` a correlated enrollment document
/// carries. A malformed document counts as uncorrelated rather than
/// failing the probe — the adapter prunes it on its own path.
fn browser_instance_of(document: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct CorrelatedDocument {
        #[serde(default)]
        browser_instance_id: Option<String>,
    }
    serde_json::from_str::<CorrelatedDocument>(document)
        .ok()
        .and_then(|document| document.browser_instance_id)
        .filter(|id| !id.is_empty() && id.len() <= 128)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correlated_document_yields_instance_id() {
        let document = r#"{"keys":{"p256dh":"k","auth":"a"},"browser_instance_id":"bi-1"}"#;
        assert_eq!(browser_instance_of(document).as_deref(), Some("bi-1"));
    }

    #[test]
    fn legacy_and_malformed_documents_are_uncorrelated() {
        assert_eq!(
            browser_instance_of(r#"{"keys":{"p256dh":"k","auth":"a"}}"#),
            None
        );
        assert_eq!(browser_instance_of("not json"), None);
        assert_eq!(
            browser_instance_of(r#"{"browser_instance_id":""}"#),
            None,
            "empty ids never correlate"
        );
    }
}
