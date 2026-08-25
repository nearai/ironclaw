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

/// Query-value encoding for the package id in a setup link.
///
/// Load-bearing, not defensive habit. `LifecyclePackageId` bounds length and
/// rejects NUL/control characters only
/// (`ironclaw_extension_contracts::lifecycle_id::validate_lifecycle_string`) —
/// `&`, `#`, `=`, and spaces all pass. A hosted-MCP package carries the
/// `desired_id` the *model* supplied at registration, so an unencoded id could
/// append a parameter to this link or truncate it. Mirrors the sibling encoder
/// in `channel_host::CONNECT_QUERY_VALUE`.
const SETUP_QUERY_VALUE: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// The web-app URL that opens this extension's personal-account setup, when the
/// deployment published a public origin.
///
/// **Why this route and not `/chat?connect=`.** The OAuth connect link is
/// withheld from any channel that cannot deliver a reply privately
/// (`channel_host::connect_required_notice`) because it *auto-installs and
/// auto-starts* a flow — landing it in a shared room pulls bystanders into
/// someone else's setup. This link starts nothing: it opens the Extensions page
/// with one extension's configure modal on its personal-account path. A
/// bystander who clicks it authenticates as themselves and lands on their own
/// page, so it is safe to sit in a group transcript and needs no privacy gate.
///
/// Returns `None` when no public origin is configured, which keeps the
/// link-free copy exactly as it reads today rather than advertising a relative
/// path into a customer conversation.
///
/// `package_id` is the same `LifecyclePackageRef.id` the Extensions page keys
/// its cards on, so the landing resolves against the caller's own inventory
/// without a second identity to keep in sync.
pub(crate) fn personal_setup_link(base_url: Option<&str>, package_id: &str) -> Option<String> {
    let base = base_url?.trim().trim_end_matches('/');
    if base.is_empty() {
        return None;
    }
    let package_id = percent_encoding::utf8_percent_encode(package_id, SETUP_QUERY_VALUE);
    Some(format!(
        "{base}/extensions?configure={package_id}&setup=personal_account"
    ))
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

/// Appended to [`DEVICE_LINK_REQUIRED_NOTICE`] when the deployment published a
/// public origin, so the user is handed the destination instead of being told
/// to go find it.
///
/// "copied exactly" earns its place: the model rewrites this sentence, and a
/// paraphrased URL is a dead one. Without a link the copy still names the
/// destination in words, which is what every deployment without a configured
/// origin keeps.
const DEVICE_LINK_SETUP_LINK_PREFIX: &str = "Give them this link, copied exactly:";

/// A resolved device-link answer for one caller and one package: the state, and
/// the destination to hand them when that state owes a link.
///
/// The two halves travel together because either alone is a defect. A link
/// without the state gets offered to a caller who already linked — #7853's
/// shape with a click attached. A state without the link is the prose-only
/// hand-off: a user reading the guidance in a Telegram or Slack thread cannot
/// render the device-link panel there, so before this they were told to go find
/// the Extensions page unaided.
pub(crate) struct DeviceLinkGuidance {
    setup: DeviceLinkUserSetup,
    setup_link: Option<String>,
}

impl DeviceLinkGuidance {
    pub(crate) fn new(setup: DeviceLinkUserSetup, setup_link: Option<String>) -> Self {
        Self { setup, setup_link }
    }

    /// Guidance for an `Active` install's `next_step` field.
    pub(crate) fn next_step(&self) -> String {
        active_install_next_step(self.setup, self.setup_link.as_deref())
    }

    /// Guidance for a response whose payload has no `next_step` field.
    pub(crate) fn activate_notice(&self) -> Option<String> {
        activate_device_link_notice(self.setup, self.setup_link.as_deref())
    }
}

/// Guidance for an `Active` install: what, if anything, the user must still do.
fn active_install_next_step(setup: DeviceLinkUserSetup, setup_link: Option<&str>) -> String {
    match device_link_notice(setup, setup_link) {
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
fn activate_device_link_notice(
    setup: DeviceLinkUserSetup,
    setup_link: Option<&str>,
) -> Option<String> {
    match setup {
        DeviceLinkUserSetup::Required => Some(required_notice(setup_link)),
        DeviceLinkUserSetup::NotApplicable | DeviceLinkUserSetup::AlreadyLinked => None,
    }
}

/// The per-state sentence appended after [`ACTIVATION_COMPLETE`], if any.
fn device_link_notice(setup: DeviceLinkUserSetup, setup_link: Option<&str>) -> Option<String> {
    match setup {
        DeviceLinkUserSetup::NotApplicable => None,
        DeviceLinkUserSetup::Required => Some(required_notice(setup_link)),
        DeviceLinkUserSetup::AlreadyLinked => Some(DEVICE_LINK_ALREADY_LINKED_NOTICE.to_string()),
    }
}

/// [`DEVICE_LINK_REQUIRED_NOTICE`] with the destination attached when there is
/// one to attach. The link is additive: the prose that names the destination in
/// words is never replaced by it, so a deployment with no public origin loses a
/// convenience rather than the instruction.
fn required_notice(setup_link: Option<&str>) -> String {
    match setup_link {
        Some(link) => {
            format!("{DEVICE_LINK_REQUIRED_NOTICE} {DEVICE_LINK_SETUP_LINK_PREFIX} {link}")
        }
        None => DEVICE_LINK_REQUIRED_NOTICE.to_string(),
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
        let none = active_install_next_step(DeviceLinkUserSetup::NotApplicable, None);
        assert_eq!(none, ACTIVATION_COMPLETE);

        let required = active_install_next_step(DeviceLinkUserSetup::Required, None);
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

        let linked = active_install_next_step(DeviceLinkUserSetup::AlreadyLinked, None);
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
            activate_device_link_notice(DeviceLinkUserSetup::Required, None),
            Some(DEVICE_LINK_REQUIRED_NOTICE.to_string())
        );
        assert_eq!(
            activate_device_link_notice(DeviceLinkUserSetup::NotApplicable, None),
            None
        );
        assert_eq!(
            activate_device_link_notice(DeviceLinkUserSetup::AlreadyLinked, None),
            None
        );
    }

    /// The hand-off this link exists to close: a user reading the guidance in a
    /// Telegram or Slack thread cannot render the device-link panel there, and
    /// before #7853's follow-up work they were handed prose and left to find
    /// the Extensions page unaided.
    #[test]
    fn a_configured_origin_hands_the_user_the_destination() {
        let link = personal_setup_link(Some("https://app.example.com"), "telegram")
            .expect("origin configured");
        assert_eq!(
            link,
            "https://app.example.com/extensions?configure=telegram&setup=personal_account"
        );

        let required = active_install_next_step(DeviceLinkUserSetup::Required, Some(&link));
        assert!(required.contains(&link), "{required}");
        // Additive, never a replacement: the words that name the destination
        // survive so the sentence still reads correctly if the URL is stripped.
        assert!(required.contains("web app"), "{required}");
        assert!(required.contains("cannot run from chat"), "{required}");
        // The model paraphrases this; a rewritten URL is a dead one.
        assert!(required.contains("copied exactly"), "{required}");
    }

    /// The link is a convenience layered on copy that already works. A
    /// deployment that published no public origin must keep the link-free
    /// sentence rather than advertise a relative path into a conversation —
    /// the failure `connect_link_base_url_from_env` documents for its sibling.
    #[test]
    fn no_configured_origin_keeps_the_link_free_copy() {
        assert_eq!(personal_setup_link(None, "telegram"), None);
        assert_eq!(personal_setup_link(Some(""), "telegram"), None);
        assert_eq!(personal_setup_link(Some("   "), "telegram"), None);
        assert_eq!(personal_setup_link(Some("/"), "telegram"), None);

        assert_eq!(
            active_install_next_step(DeviceLinkUserSetup::Required, None),
            format!("{ACTIVATION_COMPLETE} {DEVICE_LINK_REQUIRED_NOTICE}")
        );
    }

    /// A trailing slash on the origin must not produce `//extensions`, and an
    /// id carrying an `&` — which `LifecyclePackageId` permits — must not be
    /// able to append a parameter to the link or re-target it.
    #[test]
    fn the_link_is_built_defensively() {
        assert_eq!(
            personal_setup_link(Some("https://app.example.com/"), "telegram"),
            Some(
                "https://app.example.com/extensions?configure=telegram&setup=personal_account"
                    .to_string()
            )
        );

        // `LifecyclePackageId` admits this: it bounds length and rejects
        // NUL/control characters, nothing else. A hosted-MCP registration takes
        // its `desired_id` from the model.
        let link = personal_setup_link(Some("https://app.example.com"), "a&setup=workspace_bot")
            .expect("origin configured");
        assert_eq!(
            link,
            "https://app.example.com/extensions?configure=a%26setup%3Dworkspace_bot&setup=personal_account"
        );
    }

    /// Only the state that owes a link carries one. Handing a
    /// setup URL to an already-linked caller would send them to redo a
    /// ceremony they finished — the #7853 shape, with a click attached.
    #[test]
    fn only_the_owing_state_carries_the_link() {
        let link = "https://app.example.com/extensions?configure=telegram&setup=personal_account";
        let linked = active_install_next_step(DeviceLinkUserSetup::AlreadyLinked, Some(link));
        assert!(!linked.contains(link), "{linked}");

        let none = active_install_next_step(DeviceLinkUserSetup::NotApplicable, Some(link));
        assert_eq!(none, ACTIVATION_COMPLETE);

        assert_eq!(
            activate_device_link_notice(DeviceLinkUserSetup::AlreadyLinked, Some(link)),
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
