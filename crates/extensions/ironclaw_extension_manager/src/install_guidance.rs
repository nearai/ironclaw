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
use ironclaw_host_api::{
    decision::RuntimeCredentialAuthRequirement, dispatch::CredentialStageError,
    resource::ResourceScope,
};

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
    /// The package declares one or more device-link facets and this caller
    /// has not satisfied all of them.
    Required,
    /// The package declares one or more device-link facets and this caller
    /// has already satisfied every one of them.
    AlreadyLinked,
    /// The package declares one or more device-link facets and this caller's
    /// link state could not be read.
    ///
    /// Distinct from [`Self::Required`] because the credential contract makes
    /// the distinction: `CredentialStageError::Backend` is documented as
    /// "not attributable to the user's credentials", and the activation gate
    /// already maps it to a transient failure rather than missing auth
    /// (`map_activation_credential_stage_error`, pinned by
    /// `credential_staging_separates_missing_auth_from_a_credential_store_outage`).
    /// Collapsing it into `Required` is the outage half of exactly what that
    /// test forbids: sending a linked user to reconnect an account they
    /// already connected.
    Unverified,
}

/// Classify a package's device-link requirements against *this* caller.
///
/// `requirements` comes from
/// `ExtensionLifecycleManager::device_link_user_setup_requirements`, which
/// answers the package-shaped half ("what device-link surfaces exist at
/// all") — every distinct facet the package declares, not just one. This
/// resolves the caller-shaped half against the same credential-account
/// service the activation gate uses, so an already-linked user is not told to
/// link again.
///
/// `AlreadyLinked` requires every requirement in the set to be satisfied. A
/// package may declare more than one distinct device-link facet (its channel
/// connection and a separate personal-account tool credential, under
/// different providers) — collapsing to just one and reporting `AlreadyLinked`
/// from that one alone is the false-completion shape #7853 exists to remove,
/// just with a second facet still outstanding.
///
/// Every unknown resolves to [`DeviceLinkUserSetup::Required`]. That direction
/// is deliberate: "each user links their own account in the Web UI" stays true
/// when we cannot tell, whereas guessing `AlreadyLinked` reproduces #7853 by
/// letting the model report a connection complete that never happened.
pub(crate) async fn resolve_device_link_user_setup(
    requirements: Vec<RuntimeCredentialAuthRequirement>,
    credential_accounts: Option<&Arc<dyn RuntimeCredentialAccountSelectionService>>,
    scope: &ResourceScope,
) -> DeviceLinkUserSetup {
    if requirements.is_empty() {
        return DeviceLinkUserSetup::NotApplicable;
    }
    let Some(credential_accounts) = credential_accounts else {
        // Product auth is not composed on this build, so caller link state is
        // unknowable rather than absent — and `Unverified` is what says that.
        return DeviceLinkUserSetup::Unverified;
    };
    let gate = RuntimeExtensionActivationCredentialGate::new(
        scope.clone(),
        Arc::clone(credential_accounts),
    );
    match gate.missing_requirements(requirements).await {
        Ok(missing) if missing.is_empty() => DeviceLinkUserSetup::AlreadyLinked,
        Ok(_) => DeviceLinkUserSetup::Required,
        // Total-match formality, not a live path: `configured_runtime_credential_account`
        // folds an `AuthRequired` account error into `Ok(None)`, so it arrives
        // above as a *missing* requirement. If that ever changes, `Required`
        // stays the right reading of "missing, expired, or revoked".
        Err(CredentialStageError::AuthRequired) => DeviceLinkUserSetup::Required,
        // The one error that actually reaches here, and the contract documents
        // it as "not attributable to the user's credentials". Both guesses are
        // wrong, so the copy says so instead of picking one.
        Err(CredentialStageError::Backend) => {
            // silent-ok: guidance must never fail a lifecycle operation that
            // otherwise succeeded, and the outage is now stated in the
            // response rather than hidden. `warn!` is unavailable on this
            // path — it corrupts the REPL TUI (CLAUDE.md).
            tracing::debug!("device-link caller state could not be read; reporting it unverified");
            DeviceLinkUserSetup::Unverified
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
     one-time code, or for the account's two-step verification passphrase. Tell them what is \
     still missing and where to finish it; do not report the connection as complete.";

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
/// hand-off: a user reading the guidance in a chat channel cannot render the
/// device-link panel there — only the web app can — so before this they were
/// told to go find the Extensions page unaided.
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

/// The sentence for a caller whose link state could not be read.
///
/// It must not resolve the ambiguity in either direction. Claiming the link is
/// owed sends an already-linked user to redo it; claiming it is done is #7853.
/// So the copy states the uncertainty and hands over the page, which is useful
/// either way and asserts nothing.
const DEVICE_LINK_UNVERIFIED_NOTICE: &str = "Whether this user has linked their personal account could not be read just now. Do \
     not state either way: do not report the connection as complete, and do not tell them to link \
     as though it were missing. Say the check did not complete, and point them at the extension's \
     page in the web app.";

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
        DeviceLinkUserSetup::Unverified => Some(unverified_notice(setup_link)),
        DeviceLinkUserSetup::NotApplicable | DeviceLinkUserSetup::AlreadyLinked => None,
    }
}

/// The per-state sentence appended after [`ACTIVATION_COMPLETE`], if any.
fn device_link_notice(setup: DeviceLinkUserSetup, setup_link: Option<&str>) -> Option<String> {
    match setup {
        DeviceLinkUserSetup::NotApplicable => None,
        DeviceLinkUserSetup::Required => Some(required_notice(setup_link)),
        DeviceLinkUserSetup::AlreadyLinked => Some(DEVICE_LINK_ALREADY_LINKED_NOTICE.to_string()),
        DeviceLinkUserSetup::Unverified => Some(unverified_notice(setup_link)),
    }
}

/// [`DEVICE_LINK_UNVERIFIED_NOTICE`] with the destination attached when there is
/// one to attach. The link is additive: the prose that names the destination in
/// words is never replaced by it, so a deployment with no public origin loses a
/// convenience rather than the instruction.
fn unverified_notice(setup_link: Option<&str>) -> String {
    with_setup_link(DEVICE_LINK_UNVERIFIED_NOTICE, setup_link)
}

fn required_notice(setup_link: Option<&str>) -> String {
    with_setup_link(DEVICE_LINK_REQUIRED_NOTICE, setup_link)
}

/// Appends the destination when there is one, for any notice that wants it.
///
/// Shared by both notices that point at the page: the link is additive in the
/// same way for each, and a second copy of this two-line match is how the copy
/// drifted apart the first time.
fn with_setup_link(notice: &str, setup_link: Option<&str>) -> String {
    match setup_link {
        Some(link) => format!("{notice} {DEVICE_LINK_SETUP_LINK_PREFIX} {link}"),
        None => notice.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{
        capability::RuntimeCredentialAccountSetup,
        ids::{InvocationId, SecretHandle, UserId, VendorId},
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

    /// Every notice must survive the model-observation scrub that
    /// `ironclaw_threads` applies to untrusted tool output
    /// (`SENSITIVE_OBSERVATION_MARKERS` in `tool_result_reference.rs`).
    ///
    /// Found live, not by a test: an earlier revision said "an account
    /// password", and the model received "an account [redacted]" — the reason
    /// the link cannot run from chat was mangled before the LLM ever read it,
    /// and the model dropped that half of the sentence from its reply.
    ///
    /// Lifecycle `next_step` deliberately rides the UNTRUSTED channel: the
    /// trusted one is reserved for reasons built entirely from host-authored
    /// constants, and widening it is guarded by
    /// `model_influenced_invalid_binding_reason_never_reaches_the_trusted_channel`.
    /// So the copy has to avoid the vocabulary, not the scan.
    #[test]
    fn no_notice_contains_vocabulary_the_observation_scrub_would_redact() {
        // Mirrors `SENSITIVE_OBSERVATION_MARKERS`. Kept as a local list on
        // purpose: `ironclaw_threads` is not a dependency of this crate, and a
        // copy that drifts fails open here rather than silently shipping
        // mangled guidance.
        const MARKERS: [&str; 12] = [
            "access token",
            "api key",
            "api_key",
            "apikey",
            "authorization:",
            "bearer ",
            "client_secret",
            "password",
            "passwd",
            "private key",
            "raw credential",
            "secret",
        ];
        let link = "https://webui.test/extensions?configure=fixture&setup=personal_account";
        for setup in [
            DeviceLinkUserSetup::NotApplicable,
            DeviceLinkUserSetup::Required,
            DeviceLinkUserSetup::AlreadyLinked,
            DeviceLinkUserSetup::Unverified,
        ] {
            for rendered in [
                active_install_next_step(setup, Some(link)),
                active_install_next_step(setup, None),
                activate_device_link_notice(setup, Some(link)).unwrap_or_default(),
            ] {
                let lowered = rendered.to_ascii_lowercase();
                for marker in MARKERS {
                    assert!(
                        !lowered.contains(marker),
                        "{setup:?} guidance contains {marker:?}, which the model-observation \
                         scrub replaces with [redacted] before the model reads it: {rendered}"
                    );
                }
            }
        }
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
            resolve_device_link_user_setup(Vec::new(), None, &scope()).await,
            DeviceLinkUserSetup::NotApplicable
        );
    }

    /// Product auth not composed means caller link state is unknowable — which
    /// is neither "linked" nor "not linked", and the enum has a state that says
    /// exactly that. Reporting `Required` here would send an already-linked
    /// user to redo the link; reporting `AlreadyLinked` would let the model
    /// report a connection complete that never happened. Both are false.
    #[tokio::test]
    async fn unreadable_caller_state_is_reported_unverified_not_guessed() {
        assert_eq!(
            resolve_device_link_user_setup(vec![device_link_requirement()], None, &scope()).await,
            DeviceLinkUserSetup::Unverified
        );
    }

    /// The `Unverified` copy must resolve the ambiguity in NEITHER direction.
    /// A notice that leans either way is a confident false statement about
    /// something the host just failed to read.
    #[test]
    fn unverified_copy_commits_to_neither_link_state() {
        let link = "https://webui.test/extensions?configure=fixture&setup=personal_account";
        let notice = active_install_next_step(DeviceLinkUserSetup::Unverified, Some(link));

        assert!(notice.contains("could not be read"), "{notice}");
        // Hands over the page — useful either way, asserts nothing.
        assert!(notice.contains(link), "{notice}");
        // Must not borrow the Required copy's directive: that sentence tells
        // the model the link IS owed.
        assert!(
            !notice.contains("additionally requires them to"),
            "must not assert the link is owed: {notice}"
        );
        // Must not borrow the AlreadyLinked claim either.
        assert!(
            !notice.contains("already linked"),
            "must not assert the link is done: {notice}"
        );
    }

    /// A credential store that answers but hands back no usable secret is an
    /// outage, not an absent credential: `configured_runtime_credential_account`
    /// turns it into `CredentialStageError::Backend`, which the contract
    /// documents as "not attributable to the user's credentials". The
    /// activation gate already refuses to collapse the two
    /// (`credential_staging_separates_missing_auth_from_a_credential_store_outage`);
    /// reporting `Required` here would be the outage half of what that test
    /// forbids — sending a linked user to reconnect an account they already
    /// connected.
    #[tokio::test]
    async fn a_credential_store_outage_is_not_reported_as_a_missing_link() {
        let accounts: Arc<dyn RuntimeCredentialAccountSelectionService> =
            Arc::new(SecretlessRuntimeCredentialAccounts);
        assert_eq!(
            resolve_device_link_user_setup(
                vec![device_link_requirement()],
                Some(&accounts),
                &scope(),
            )
            .await,
            DeviceLinkUserSetup::Unverified,
            "an unreadable credential store must not be reported as an absent credential"
        );
    }

    /// Answers every lookup with an account carrying no access secret, which
    /// `configured_runtime_credential_account` classifies as `Backend`.
    struct SecretlessRuntimeCredentialAccounts;

    #[async_trait::async_trait]
    impl RuntimeCredentialAccountSelectionService for SecretlessRuntimeCredentialAccounts {
        async fn select_unique_configured_runtime_account(
            &self,
            _request: ironclaw_auth::RuntimeCredentialAccountSelectionRequest,
        ) -> Result<ironclaw_auth::CredentialAccount, ironclaw_auth::AuthProductError> {
            let mut account = fake_configured_credential_account();
            account.access_secret = None;
            Ok(account)
        }

        async fn select_configured_account_for_binding(
            &self,
            _lookup: ironclaw_auth::CredentialAccountSelectionRequest,
            _runtime_scope: ironclaw_auth::AuthProductScope,
        ) -> Result<ironclaw_auth::CredentialAccount, ironclaw_auth::AuthProductError> {
            Err(ironclaw_auth::AuthProductError::CredentialMissing)
        }
    }

    /// A fake [`RuntimeCredentialAccountSelectionService`] that satisfies
    /// exactly the first `satisfied` requirements it is asked about, in call
    /// order, and reports every later one missing. `RuntimeCredentialAccountSelectionRequest`
    /// exposes no accessor a caller outside `ironclaw_auth` can read, so a
    /// fake here cannot branch on *which* provider a call names — call order
    /// is the only lever available, and the test below controls that order by
    /// building the requirement vector itself.
    struct SatisfyFirstNRuntimeCredentialAccounts {
        remaining_satisfied: std::sync::atomic::AtomicUsize,
    }

    impl SatisfyFirstNRuntimeCredentialAccounts {
        fn new(satisfied: usize) -> Self {
            Self {
                remaining_satisfied: std::sync::atomic::AtomicUsize::new(satisfied),
            }
        }
    }

    #[async_trait::async_trait]
    impl RuntimeCredentialAccountSelectionService for SatisfyFirstNRuntimeCredentialAccounts {
        async fn select_unique_configured_runtime_account(
            &self,
            _request: ironclaw_auth::RuntimeCredentialAccountSelectionRequest,
        ) -> Result<ironclaw_auth::CredentialAccount, ironclaw_auth::AuthProductError> {
            use std::sync::atomic::Ordering;
            let satisfied = self
                .remaining_satisfied
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok();
            if satisfied {
                Ok(fake_configured_credential_account())
            } else {
                Err(ironclaw_auth::AuthProductError::CredentialMissing)
            }
        }

        async fn select_configured_account_for_binding(
            &self,
            _lookup: ironclaw_auth::CredentialAccountSelectionRequest,
            _runtime_scope: ironclaw_auth::AuthProductScope,
        ) -> Result<ironclaw_auth::CredentialAccount, ironclaw_auth::AuthProductError> {
            // Unused by the activation gate; the device-link guidance path
            // only ever calls the runtime-resolution half above.
            Err(ironclaw_auth::AuthProductError::CredentialMissing)
        }
    }

    fn fake_configured_credential_account() -> ironclaw_auth::CredentialAccount {
        let now = chrono::Utc::now();
        ironclaw_auth::CredentialAccount {
            id: ironclaw_auth::CredentialAccountId::new(),
            scope: ironclaw_auth::AuthProductScope::new(scope(), ironclaw_auth::AuthSurface::Api),
            provider: ironclaw_auth::AuthProviderId::new("fixture-vendor").expect("provider id"),
            label: ironclaw_auth::CredentialAccountLabel::new("fixture account")
                .expect("account label"),
            status: ironclaw_auth::CredentialAccountStatus::Configured,
            ownership: ironclaw_auth::CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("fixture_secret").expect("secret handle")),
            refresh_secret: None,
            scopes: Vec::new(),
            provider_identity: None,
            link_revision: 0,
            created_at: now,
            updated_at: now,
        }
    }

    /// The #7853 follow-up this module exists to prevent: a package can
    /// declare two DISTINCT device-link requirements (its channel connection
    /// and a separate personal-account tool credential, under different
    /// providers — nothing ties the two together). A caller who satisfied
    /// only the first must be reported `Required`, never `AlreadyLinked` —
    /// `AlreadyLinked` here would tell the model the personal-account tools
    /// will work when a required link is still missing.
    #[tokio::test]
    async fn two_distinct_requirements_with_only_one_satisfied_never_reports_already_linked() {
        let channel_requirement = device_link_requirement();
        let tool_requirement = RuntimeCredentialAuthRequirement {
            provider: VendorId::new("fixture-tool-vendor").expect("vendor"),
            ..channel_requirement.clone()
        };
        assert_ne!(
            channel_requirement, tool_requirement,
            "the two facets must be distinct for this test to prove anything"
        );

        let credential_accounts: Arc<dyn RuntimeCredentialAccountSelectionService> =
            Arc::new(SatisfyFirstNRuntimeCredentialAccounts::new(1));

        let setup = resolve_device_link_user_setup(
            vec![channel_requirement, tool_requirement],
            Some(&credential_accounts),
            &scope(),
        )
        .await;

        assert_eq!(
            setup,
            DeviceLinkUserSetup::Required,
            "one satisfied facet out of two must not report AlreadyLinked"
        );
    }
}
