use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use ironclaw_auth::{
    AuthProductError, AuthProductScope, AuthProviderId, AuthSurface, CredentialAccount,
    CredentialAccountId, CredentialAccountLabel, CredentialAccountSelectionRequest,
    CredentialAccountStatus, CredentialOwnership, RuntimeCredentialAccountSelectionRequest,
};
use ironclaw_extension_registry::{
    ExtensionManifest, ExtensionManifestRecord, ExtensionPackage, ExtensionRegistry, ManifestSource,
};
use ironclaw_host_api::{
    ids::SecretHandle,
    ids::{AgentId, InvocationId, ProjectId, TenantId, ThreadId},
    path::VirtualPath,
};
use ironclaw_loop_contracts::{
    InMemoryRunProfileResolver, RunProfileResolutionRequest, RunProfileResolver,
};
use ironclaw_turns::{TurnId, TurnRunId, TurnScope};

/// One extension with a credential-free tool and two tools that share one
/// mandatory product-auth account.
const FIXTURE_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "fixture"
name = "Fixture Extension"
version = "0.1.0"
description = "Tool availability fixture"
trust = "first_party_requested"

[runtime]
kind = "wasm"
module = "wasm/fixture.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "fixture.search"
description = "Search without an account"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/search.input.json"
output_schema_ref = "schemas/search.output.json"

[[capability_provider.tools.capabilities]]
id = "fixture.send"
description = "Send with a connected account"
effects = ["network", "use_secret"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/search.input.json"
output_schema_ref = "schemas/search.output.json"

[[capability_provider.tools.capabilities.runtime_credentials]]
handle = "fixture_account"
source = { type = "product_auth_account", provider = "google" }
audience = { scheme = "https", host_pattern = "api.example.com" }
target = { type = "header", name = "authorization" }
required = true

[[capability_provider.tools.capabilities]]
id = "fixture.read"
description = "Read with a connected account"
effects = ["network", "use_secret"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/search.input.json"
output_schema_ref = "schemas/search.output.json"

[[capability_provider.tools.capabilities.runtime_credentials]]
handle = "fixture_account"
source = { type = "product_auth_account", provider = "google" }
audience = { scheme = "https", host_pattern = "api.example.com" }
target = { type = "header", name = "authorization" }
required = true
"#;

fn registry() -> Arc<SharedExtensionRegistry> {
    let contracts = crate::product_extension_host_api_contract_registry().expect("contracts");
    let root = VirtualPath::new("/system/extensions/fixture").expect("extension root");
    let record = ExtensionManifestRecord::from_toml(
        FIXTURE_MANIFEST,
        ManifestSource::HostBundled,
        &ironclaw_host_api::host_port::default_host_port_catalog().expect("host ports"),
        None,
        &contracts,
        Some(root.clone()),
    )
    .expect("fixture manifest");
    let manifest = ExtensionManifest::try_from(record.manifest().clone()).expect("package view");
    let package =
        ExtensionPackage::from_manifest_toml(manifest, root, FIXTURE_MANIFEST).expect("package");
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).expect("registered");
    Arc::new(SharedExtensionRegistry::new(registry))
}

/// How the fake credential store answers every lookup. Its request type
/// exposes nothing to branch on, so one store answers all lookups alike.
#[derive(Debug, Clone, Copy)]
enum Store {
    Configured,
    AccountMissing,
    /// Answers with an account that has no secret, which the lookup reads as
    /// a backend failure.
    Outage,
    Hangs,
}

struct FakeAccounts {
    store: Store,
    lookups: AtomicUsize,
}

impl FakeAccounts {
    fn new(store: Store) -> Arc<Self> {
        Arc::new(Self {
            store,
            lookups: AtomicUsize::new(0),
        })
    }
}

fn configured_account(has_secret: bool) -> CredentialAccount {
    let now = chrono::Utc::now();
    let scope =
        ResourceScope::local_default(UserId::new("user").expect("user"), InvocationId::new())
            .expect("scope");
    CredentialAccount {
        id: CredentialAccountId::new(),
        scope: AuthProductScope::new(scope, AuthSurface::Api),
        provider: AuthProviderId::new("google").expect("provider id"),
        label: CredentialAccountLabel::new("fixture account").expect("label"),
        status: CredentialAccountStatus::Configured,
        ownership: CredentialOwnership::UserReusable,
        owner_extension: None,
        granted_extensions: Vec::new(),
        access_secret: has_secret.then(|| SecretHandle::new("fixture_secret").expect("secret")),
        refresh_secret: None,
        scopes: Vec::new(),
        provider_identity: None,
        link_revision: 0,
        created_at: now,
        updated_at: now,
    }
}

#[async_trait]
impl RuntimeCredentialAccountSelectionService for FakeAccounts {
    async fn select_unique_configured_runtime_account(
        &self,
        _request: RuntimeCredentialAccountSelectionRequest,
    ) -> Result<CredentialAccount, AuthProductError> {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        match self.store {
            Store::Configured => Ok(configured_account(true)),
            Store::AccountMissing => Err(AuthProductError::CredentialMissing),
            Store::Outage => Ok(configured_account(false)),
            Store::Hangs => {
                tokio::time::sleep(Duration::from_secs(3_600)).await;
                Ok(configured_account(true))
            }
        }
    }

    async fn select_configured_account_for_binding(
        &self,
        _lookup: CredentialAccountSelectionRequest,
        _runtime_scope: AuthProductScope,
    ) -> Result<CredentialAccount, AuthProductError> {
        Err(AuthProductError::CredentialMissing)
    }
}

async fn run_context() -> LoopRunContext {
    let scope = TurnScope::new(
        TenantId::new("tenant").expect("tenant"),
        Some(AgentId::new("agent").expect("agent")),
        Some(ProjectId::new("project").expect("project")),
        ThreadId::new("thread").expect("thread"),
    );
    let profile = InMemoryRunProfileResolver::default()
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .expect("profile");
    LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), profile)
}

fn ids(names: &[&str]) -> Vec<CapabilityId> {
    names
        .iter()
        .map(|name| CapabilityId::new(*name).expect("capability id"))
        .collect()
}

async fn answers(store: Store) -> (BTreeMap<String, ToolAvailability>, usize) {
    let accounts = FakeAccounts::new(store);
    let predicate = extension_tool_availability(
        registry(),
        Arc::clone(&accounts) as Arc<dyn RuntimeCredentialAccountSelectionService>,
        UserId::new("owner").expect("user"),
    );
    let answers = predicate
        .availability(
            &run_context().await,
            &ids(&[
                "fixture.search",
                "fixture.send",
                "fixture.read",
                "builtin.result_read",
            ]),
        )
        .await
        .into_iter()
        .map(|(id, answer)| (id.as_str().to_string(), answer))
        .collect();
    (answers, accounts.lookups.load(Ordering::SeqCst))
}

const MISSING: ToolAvailability =
    ToolAvailability::Unavailable(ToolUnavailableReason::CredentialMissing);

fn answer(answers: &BTreeMap<String, ToolAvailability>, id: &str) -> ToolAvailability {
    answers.get(id).copied().expect("an answer for every id")
}

#[tokio::test]
async fn a_missing_account_makes_its_tools_unavailable() {
    let (answers, lookups) = answers(Store::AccountMissing).await;
    assert_eq!(answer(&answers, "fixture.send"), MISSING);
    assert_eq!(answer(&answers, "fixture.read"), MISSING);
    assert_eq!(lookups, 1, "a shared requirement is looked up once");
}

#[tokio::test]
async fn a_configured_account_makes_its_tools_available() {
    let (answers, _) = answers(Store::Configured).await;
    assert_eq!(
        answer(&answers, "fixture.send"),
        ToolAvailability::Available
    );
    assert_eq!(
        answer(&answers, "fixture.read"),
        ToolAvailability::Available
    );
}

#[tokio::test]
async fn tools_needing_no_credential_are_available_whatever_the_store_says() {
    for store in [Store::AccountMissing, Store::Outage, Store::Configured] {
        let (answers, _) = answers(store).await;
        assert_eq!(
            answer(&answers, "fixture.search"),
            ToolAvailability::Available,
            "{store:?}"
        );
        assert_eq!(
            answer(&answers, "builtin.result_read"),
            ToolAvailability::Available,
            "a host tool outside the extension registry declares no account ({store:?})"
        );
    }
}

#[tokio::test]
async fn a_store_outage_is_unknown_not_missing() {
    let (answers, _) = answers(Store::Outage).await;
    assert_eq!(answer(&answers, "fixture.send"), ToolAvailability::Unknown);
}

#[tokio::test(start_paused = true)]
async fn a_lookup_past_its_deadline_is_unknown() {
    let started = tokio::time::Instant::now();
    let (answers, _) = answers(Store::Hangs).await;
    assert_eq!(answer(&answers, "fixture.send"), ToolAvailability::Unknown);
    assert_eq!(
        answer(&answers, "fixture.search"),
        ToolAvailability::Available
    );
    assert!(
        started.elapsed() < TOOL_AVAILABILITY_LOOKUP_TIMEOUT + Duration::from_millis(50),
        "the lookup gives up at its deadline"
    );
}
