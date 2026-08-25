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
/// It is deliberately worded to stop the model short of reporting success: the
/// ceremony renders a scannable code and asks for a one-time code and an
/// account password, so it cannot run from chat on any surface, and claiming
/// the connection is finished is exactly the false completion #7853 describes.
///
/// Both renderers below compose this rather than restating it. An earlier
/// revision of this module spelled it out twice — the same drift shape the
/// module exists to remove.
const DEVICE_LINK_REQUIRED_NOTICE: &str = "Personal capabilities and chatting as the user still require each user to link their own \
     account from this extension's card in the Web UI — that ceremony cannot run from chat, so \
     direct the user there rather than reporting the connection complete.";

/// The sentence for a caller who has already finished the ceremony.
const DEVICE_LINK_ALREADY_LINKED_NOTICE: &str = "The calling user's own account is already linked for this extension, so personal \
     capabilities are available — do not ask them to link, scan, or authorize it again.";

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
