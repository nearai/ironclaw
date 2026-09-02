//! Tests for `run_completion_push`; a sibling file so the composition mass gate
//! counts production lines only (`scripts/ci/composition-budget.toml`).

use super::*;
use ironclaw_extension_contracts::channel_adapter::DeliveryRegistration;
use ironclaw_host_api::ids::{TenantId, UserId};
use ironclaw_product_contracts::delivery::{
    DeliveryRegistrationError, DeliveryRegistrationRequest,
};

/// Keys shaped like a real `PushManager.subscribe()` result: a 65-byte
/// uncompressed P-256 point and a 16-byte auth secret, base64url.
fn valid_keys() -> String {
    use base64::Engine as _;
    let mut point = vec![0x04u8];
    point.extend(std::iter::repeat_n(0x11u8, 64));
    let p256dh = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(point);
    let auth = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([0x22u8; 16]);
    format!(r#""keys":{{"p256dh":"{p256dh}","auth":"{auth}"}}"#)
}

struct ScriptedRegistrations(Vec<DeliveryRegistration>);

#[async_trait]
impl DeliveryRegistrationService for ScriptedRegistrations {
    async fn list(
        &self,
        _scope: &DeliveryRegistrationScope,
    ) -> Result<Vec<DeliveryRegistration>, DeliveryRegistrationError> {
        Ok(self.0.clone())
    }

    async fn enroll(
        &self,
        _scope: &DeliveryRegistrationScope,
        _request: DeliveryRegistrationRequest,
    ) -> Result<DeliveryRegistration, DeliveryRegistrationError> {
        unreachable!("the probe only lists")
    }

    async fn remove(
        &self,
        _scope: &DeliveryRegistrationScope,
        _registration_id: &str,
    ) -> Result<bool, DeliveryRegistrationError> {
        unreachable!("the probe only lists")
    }

    async fn prune(
        &self,
        _scope: &DeliveryRegistrationScope,
        _registration_ids: &[String],
    ) -> Result<usize, DeliveryRegistrationError> {
        unreachable!("the probe only lists")
    }
}

fn registration(id: &str, document: String) -> DeliveryRegistration {
    DeliveryRegistration {
        registration_id: id.to_string(),
        endpoint: "https://push.example/send/a".to_string(),
        document,
        created_at: "2026-08-13T00:00:00Z".to_string(),
    }
}

#[tokio::test]
async fn enrollment_correlates_through_the_web_app_document_grammar() {
    let keys = valid_keys();
    let probe = RegistrationEnrollmentProbe::new(
        Arc::new(ScriptedRegistrations(vec![
            registration(
                "correlated",
                format!(r#"{{{keys},"browser_instance_id":"rbi-1"}}"#),
            ),
            registration("legacy", format!("{{{keys}}}")),
            // Not a push subscription at all: the adapter would prune it,
            // so the probe must not count it as presence either.
            registration(
                "malformed",
                r#"{"browser_instance_id":"rbi-2"}"#.to_string(),
            ),
        ])),
        ExtensionId::new("web-app").expect("extension id"),
    );
    let snapshot = probe
        .enrollment(&RunCompletionOwner {
            tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
            user_id: UserId::new("user-alpha").expect("user"),
        })
        .await
        .expect("probe lists");
    assert_eq!(snapshot.instance_ids, vec!["rbi-1".to_string()]);
    assert_eq!(
        snapshot.uncorrelated, 1,
        "legacy records degrade to presence"
    );
}
