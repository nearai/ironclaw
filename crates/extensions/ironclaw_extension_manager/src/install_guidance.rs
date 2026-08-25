//! Model-visible guidance for an install/activate that reached `Active`.
//!
//! Both lifecycle surfaces — the agent-callable capability handler
//! (`extension_lifecycle_capabilities`) and the WebUI-facing product service
//! (`lifecycle_product_service`) — render the same sentences from the same
//! state, so the copy and the branch live here once. They were byte-identical
//! copies in both files until #7853; drift between synchronized copies is the
//! defect class that issue belongs to, so a single owner is the point of this
//! module rather than a nicety.

use std::sync::Arc;

use ironclaw_auth::RuntimeCredentialAccountSelectionService;
use ironclaw_extension_host::extension_activation_credentials::RuntimeExtensionActivationCredentialGate;
use ironclaw_host_api::{decision::RuntimeCredentialAuthRequirement, resource::ResourceScope};

/// Whether the calling user still has a device-link ceremony to complete for
/// this package.
///
/// A device link is per user: the deployment activating an extension does not
/// link anyone's personal account, and one user linking theirs does nothing for
/// the next. The three states are distinguished because collapsing them is how
/// #7853 reached users — a package with no device-link surface must not be
/// described as needing one, and a user who has *already* linked must not be
/// sent back to the Web UI to redo it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeviceLinkUserSetup {
    /// The package declares no device-link surface on any facet.
    NotApplicable,
    /// The package declares one and this caller has not satisfied it.
    Required,
    /// The package declares one and this caller has already satisfied it.
    AlreadyLinked,
}

/// Classify a package's device-link requirement against *this* caller.
///
/// `requirement` comes from
/// `ExtensionLifecycleManager::device_link_user_setup_requirement`, which
/// answers the package-shaped half ("is there a device-link surface at all").
/// This resolves the caller-shaped half against the same credential-account
/// service the activation gate uses, so an already-linked user is not told to
/// link again.
///
/// Every unknown resolves to [`DeviceLinkUserSetup::Required`]. That direction
/// is deliberate: "each user links their own account in the Web UI" stays true
/// when we cannot tell, whereas guessing `AlreadyLinked` reproduces #7853 by
/// letting the model report a connection complete that never happened.
pub(crate) async fn resolve_device_link_user_setup(
    requirement: Option<RuntimeCredentialAuthRequirement>,
    credential_accounts: Option<&Arc<dyn RuntimeCredentialAccountSelectionService>>,
    scope: &ResourceScope,
) -> DeviceLinkUserSetup {
    let Some(requirement) = requirement else {
        return DeviceLinkUserSetup::NotApplicable;
    };
    let Some(credential_accounts) = credential_accounts else {
        // Product auth is not composed on this build, so caller link state is
        // unknowable rather than absent.
        return DeviceLinkUserSetup::Required;
    };
    let gate = RuntimeExtensionActivationCredentialGate::new(
        scope.clone(),
        Arc::clone(credential_accounts),
    );
    match gate.missing_requirements(vec![requirement]).await {
        Ok(missing) if missing.is_empty() => DeviceLinkUserSetup::AlreadyLinked,
        Ok(_) => DeviceLinkUserSetup::Required,
        Err(error) => {
            // silent-ok: guidance copy must never fail a lifecycle operation,
            // and the fail-toward-Required direction is documented above.
            tracing::debug!(
                error = ?error,
                "device-link caller state unresolved; reporting link as still required"
            );
            DeviceLinkUserSetup::Required
        }
    }
}

/// The sentence every surface renders when the calling user still owes a
/// device link, defined once.
///
/// Worded to stop the model short of reporting success, and to be safe to
/// quote: the model paraphrases this, so it states the benefit in the user's
/// terms (the manifest's own "read your chats and send messages as you") and
/// keeps the directive in its own sentence.
///
/// It gives the REASON chat cannot finish the link, not just the mechanic.
/// Naming the QR alone reads as a non-sequitur — a code you scan on your phone
/// sounds like something a chat could display. The actual constraint is that
/// the displayed value IS a login token
/// ([`DeviceLinkPayload`](ironclaw_extension_contracts::device_link::DeviceLinkPayload):
/// "anything that can display it can invite a device onto the account"), so
/// putting it in a conversation writes a bearer credential into the
/// transcript. Fusing directive into fact is how
/// a paraphrase becomes "I should direct you there rather than reporting the
/// connection complete."
///
/// Claiming the connection is finished is exactly the false completion #7853
/// describes, so the copy never does.
///
/// Both renderers below compose this rather than restating it. An earlier
/// revision of this module spelled it out twice — the same drift shape the
/// module exists to remove.
const DEVICE_LINK_REQUIRED_NOTICE: &str = "Reading this user's own chats, or sending messages as them, additionally requires them to \
     link their personal account from the extension's page in the web app. That cannot run from \
     chat: the code it displays is itself a login credential, and later steps can ask for a \
     one-time code or an account password. Tell them what is still missing and where to finish \
     it; do not report the connection as complete.";

/// The sentence for a caller who has already finished the link.
const DEVICE_LINK_ALREADY_LINKED_NOTICE: &str = "This user has already linked their personal account, so reading their chats and sending \
     messages as them will work. Do not ask them to link, scan, or authorize anything again.";

/// What an `Active` install reports when nothing further is owed.
const ACTIVATION_COMPLETE: &str = "Activation completed; model-visible extension tools are ready.";

/// Guidance for an `Active` install: what, if anything, the user must still do.
pub(crate) fn active_install_next_step(setup: DeviceLinkUserSetup) -> String {
    match device_link_notice(setup) {
        Some(notice) => format!("{ACTIVATION_COMPLETE} {notice}"),
        None => ACTIVATION_COMPLETE.to_string(),
    }
}

/// Guidance for a lifecycle response that carries no `next_step` field.
///
/// `LifecycleProductPayload::ExtensionActivate` has no place to put a next
/// step, so a bare-activate arm would silently drop the device-link guidance
/// the install arms render. This is the same sentence routed through the
/// payload-shape-independent `message` field instead, so a user who activates
/// without installing still learns where to finish. `AlreadyLinked` stays
/// silent here: an activate response is not the place to volunteer that
/// nothing is owed.
pub(crate) fn activate_device_link_notice(setup: DeviceLinkUserSetup) -> Option<&'static str> {
    match setup {
        DeviceLinkUserSetup::Required => Some(DEVICE_LINK_REQUIRED_NOTICE),
        DeviceLinkUserSetup::NotApplicable | DeviceLinkUserSetup::AlreadyLinked => None,
    }
}

/// The per-state sentence appended after [`ACTIVATION_COMPLETE`], if any.
fn device_link_notice(setup: DeviceLinkUserSetup) -> Option<&'static str> {
    match setup {
        DeviceLinkUserSetup::NotApplicable => None,
        DeviceLinkUserSetup::Required => Some(DEVICE_LINK_REQUIRED_NOTICE),
        DeviceLinkUserSetup::AlreadyLinked => Some(DEVICE_LINK_ALREADY_LINKED_NOTICE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{
        capability::RuntimeCredentialAccountSetup,
        ids::{InvocationId, UserId, VendorId},
    };

    fn device_link_requirement() -> RuntimeCredentialAuthRequirement {
        RuntimeCredentialAuthRequirement {
            provider: VendorId::new("telegram").expect("vendor"),
            setup: RuntimeCredentialAccountSetup::DeviceLink,
            requester_extension: ironclaw_host_api::ids::ExtensionId::new("telegram")
                .expect("extension"),
            provider_scopes: Vec::new(),
        }
    }

    fn scope() -> ResourceScope {
        ResourceScope::local_default(
            UserId::new("user-alpha").expect("user"),
            InvocationId::new(),
        )
        .expect("scope")
    }

    /// The three states must stay distinguishable in the copy. Collapsing
    /// `AlreadyLinked` into `Required` would tell a user who has already linked
    /// to go link again — the false-guidance class #7853 exists to remove — and
    /// the bare-activate arm's positive case cannot be driven end to end, so
    /// this is the only place that pins it.
    #[test]
    fn each_state_renders_distinct_install_guidance() {
        let none = active_install_next_step(DeviceLinkUserSetup::NotApplicable);
        assert_eq!(none, ACTIVATION_COMPLETE);

        let required = active_install_next_step(DeviceLinkUserSetup::Required);
        assert!(required.starts_with(ACTIVATION_COMPLETE), "{required}");
        // Four properties, because a prohibition without a reason reads as a
        // non-sequitur: a code you scan on your phone sounds like something a
        // chat could display, so the copy has to say why it cannot.
        assert!(
            required.contains("web app"),
            "must name where the link is finished: {required}"
        );
        assert!(
            required.contains("cannot run from chat"),
            "must say chat cannot finish it: {required}"
        );
        assert!(
            required.contains("login credential") || required.contains("account password"),
            "must give the REASON chat cannot finish it, not just the prohibition: {required}"
        );
        assert!(
            !required.contains("ceremony"),
            "internal vocabulary the model would echo verbatim: {required}"
        );

        let linked = active_install_next_step(DeviceLinkUserSetup::AlreadyLinked);
        assert!(linked.starts_with(ACTIVATION_COMPLETE), "{linked}");
        assert!(linked.contains("already linked"), "{linked}");
        assert!(
            !linked.contains("cannot run from chat"),
            "an already-linked caller must never be sent to link again: {linked}"
        );
    }

    /// The activate arms carry the notice on `message`, and only when the
    /// caller actually owes a link. `AlreadyLinked` stays silent here on
    /// purpose: an activate response is not the place to volunteer that nothing
    /// is owed.
    #[test]
    fn activate_notice_fires_only_when_a_link_is_owed() {
        assert_eq!(
            activate_device_link_notice(DeviceLinkUserSetup::Required),
            Some(DEVICE_LINK_REQUIRED_NOTICE)
        );
        assert_eq!(
            activate_device_link_notice(DeviceLinkUserSetup::NotApplicable),
            None
        );
        assert_eq!(
            activate_device_link_notice(DeviceLinkUserSetup::AlreadyLinked),
            None
        );
    }

    #[tokio::test]
    async fn no_requirement_resolves_to_not_applicable() {
        assert_eq!(
            resolve_device_link_user_setup(None, None, &scope()).await,
            DeviceLinkUserSetup::NotApplicable
        );
    }

    /// Product auth not composed means caller link state is unknowable, not
    /// absent. Failing toward `Required` keeps a true sentence on screen;
    /// guessing `AlreadyLinked` would let the model report a connection
    /// complete that never happened.
    #[tokio::test]
    async fn unresolvable_caller_state_fails_toward_required() {
        assert_eq!(
            resolve_device_link_user_setup(Some(device_link_requirement()), None, &scope()).await,
            DeviceLinkUserSetup::Required
        );
    }
}
