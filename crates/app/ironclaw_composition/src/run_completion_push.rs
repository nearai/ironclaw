//! Composition half of run-completion external presentation (2026-08-13
//! design §6.1, §7.9): the host-owned web-app enrollment probe the
//! `local_os` policy consults.
//!
//! The probe reads the SAME host-owned delivery registrations the delivery
//! coordinator resolves for actual pushes, so "Enrolled" here can never
//! diverge from what a push would use. The registration document is
//! interpreted through the web-app domain's own grammar
//! ([`RegistrationDocument`]) — the parser the delivery adapter sends with —
//! so composition wires the correlation without owning any of the document
//! shape; records that predate correlation degrade to profile-level presence.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_assistant::run_completions::push::{WebAppEnrollmentProbe, WebAppEnrollmentSnapshot};
use ironclaw_assistant::run_completions::store::RunCompletionOwner;
use ironclaw_host_api::ids::ExtensionId;
use ironclaw_product_contracts::delivery::{
    DeliveryRegistrationScope, DeliveryRegistrationService,
};
use ironclaw_web_app::RegistrationDocument;

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
            match RegistrationDocument::parse(&registration.document) {
                Ok(document) => match document.browser_instance_id {
                    Some(instance_id) => snapshot.instance_ids.push(instance_id),
                    None => snapshot.uncorrelated += 1,
                },
                Err(error) => {
                    // A document the delivery half cannot parse can never be
                    // pushed to, so it is not an enrollment either; the
                    // adapter prunes it on its own path.
                    tracing::debug!(
                        target: "ironclaw::reborn::run_completions",
                        %error,
                        "enrollment probe skipped an unparseable registration",
                    );
                }
            }
        }
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests;
