use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{Signer, SigningKey};
use ironclaw_common::hashing::sha256_hex;
use ironclaw_host_api::{
    CapabilityId, InvocationId, NetworkScheme, ResourceScope, RuntimeHttpEgress,
    RuntimeHttpEgressError, RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, RuntimeKind,
    UserId, sha256_digest_token,
};
use ironclaw_product_workflow::{LifecyclePhase, LifecycleProductPayload, LifecycleSkillSource};

use crate::RebornBuildInput;
use crate::factory::build_reborn_services;
use crate::lifecycle::response_with_payload;

use super::catalog::{
    classify_gate_and_digest, host_is_disallowed_target, skill_summary, tool_artifact_digest,
    tool_summary, verify_signed_manifest_with_keys,
};
use super::model::{
    IronHubArtifact, IronHubEntryKind, IronHubInstallOptions, IronHubManifest, IronHubProvenance,
    IronHubSkillEntry, IronHubToolEntry,
};
use super::render::render_reborn_ironhub_response;
use super::service::IronHubService;

#[test]
fn signed_manifest_verifies_known_test_vector() {
    let envelope = br#"{"v":1,"key_id":"test-vector","manifest_b64":"eyJ2ZXJzaW9uIjoiMSIsImdlbmVyYXRlZF9hdCI6IjIwMjYtMDEtMDFUMDA6MDA6MDBaIiwicmVsZWFzZV90YWciOiJ0ZXN0IiwicmVwbyI6Im5lYXJhaS9pcm9uaHViIiwidG9vbHMiOltdLCJza2lsbHMiOltdfQ","sig":"KjsUDgi1enj3iTPNQI6gU1Bwxf01hIUItlFvX9PxgWNybPPrJNIV7vFG-G8hJOalFMwFs5zQHrxbtFDZAlgtBg"}"#;
    let manifest = verify_signed_manifest_with_keys(
        envelope,
        &[(
            "test-vector",
            "ca46572f4dcd485599cdf95442934a3e3c86e2cae766a85fbffc8d6540959928",
        )],
    )
    .expect("signed manifest verifies");

    assert_eq!(
            manifest,
            br#"{"version":"1","generated_at":"2026-01-01T00:00:00Z","release_tag":"test","repo":"nearai/ironhub","tools":[],"skills":[]}"#
        );
}

#[test]
fn missing_provenance_defaults_to_unverified() {
    let manifest: IronHubManifest = serde_json::from_str(
        r#"{
                "version": "1",
                "generated_at": "2026-01-01T00:00:00Z",
                "release_tag": "test",
                "repo": "nearai/ironhub",
                "tools": [{
                    "name": "community-tool",
                    "crate_name": "community-tool",
                    "version": "0.1.0",
                    "description": "community",
                    "wasm": {
                        "url": "https://hub.ironclaw.com/community-tool.wasm",
                        "size_bytes": 1,
                        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    },
                    "capabilities": {
                        "url": "https://hub.ironclaw.com/community-tool.capabilities.json",
                        "size_bytes": 1,
                        "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                    }
                }],
                "skills": [{
                    "name": "community-skill",
                    "version": "0.1.0",
                    "description": "community",
                    "skill_md": {
                        "url": "https://hub.ironclaw.com/community-skill/SKILL.md",
                        "size_bytes": 1,
                        "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                    }
                }]
            }"#,
    )
    .expect("manifest parses");

    assert_eq!(manifest.tools[0].provenance, IronHubProvenance::New);
    assert_eq!(manifest.skills[0].provenance, IronHubProvenance::New);
}

#[test]
fn unverified_install_requires_acknowledgement() {
    let manifest = IronHubManifest {
        version: "1".to_string(),
        generated_at: "2026-01-01T00:00:00Z".to_string(),
        release_tag: "test".to_string(),
        repo: "nearai/ironhub".to_string(),
        tools: Vec::new(),
        skills: vec![IronHubSkillEntry {
            name: "community-skill".to_string(),
            trunk: String::new(),
            version: "0.1.0".to_string(),
            description: String::new(),
            provenance: IronHubProvenance::New,
            skill_md: IronHubArtifact {
                url: "https://hub.ironclaw.com/community-skill/SKILL.md".to_string(),
                size_bytes: 1,
                sha256: "c".repeat(64),
            },
        }],
    };

    let blocked = classify_gate_and_digest(
        &manifest,
        "community-skill",
        Some(IronHubEntryKind::Skill),
        &IronHubInstallOptions::default(),
    )
    .expect_err("unverified content requires acknowledgement");
    assert!(blocked.to_string().contains("UNVERIFIED community content"));

    let allowed = classify_gate_and_digest(
        &manifest,
        "community-skill",
        Some(IronHubEntryKind::Skill),
        &IronHubInstallOptions {
            acknowledge_unverified: true,
            ..IronHubInstallOptions::default()
        },
    )
    .expect("acknowledged unverified content can proceed");
    assert_eq!(allowed.0, IronHubEntryKind::Skill);
    assert_eq!(allowed.1, IronHubProvenance::New);
}

#[test]
fn renderer_includes_tools_and_skills_in_mixed_search() {
    let skill = skill_summary(&IronHubSkillEntry {
        name: "reviewer".to_string(),
        trunk: String::new(),
        version: "0.2.0".to_string(),
        description: "review skill".to_string(),
        provenance: IronHubProvenance::Verified,
        skill_md: IronHubArtifact {
            url: "https://hub.ironclaw.com/reviewer/SKILL.md".to_string(),
            size_bytes: 1,
            sha256: "c".repeat(64),
        },
    })
    .expect("skill summary");
    assert_eq!(skill.source, LifecycleSkillSource::Registry);

    let response = response_with_payload(
        None,
        LifecyclePhase::Discovered,
        LifecycleProductPayload::CatalogSearch {
            count: 2,
            tools: vec![
                tool_summary(&IronHubToolEntry {
                    name: "web".to_string(),
                    crate_name: "web-tool".to_string(),
                    version: "0.1.0".to_string(),
                    description: "web tool".to_string(),
                    provenance: IronHubProvenance::Official,
                    wasm: IronHubArtifact {
                        url: "https://hub.ironclaw.com/web.wasm".to_string(),
                        size_bytes: 1,
                        sha256: "a".repeat(64),
                    },
                    capabilities: IronHubArtifact {
                        url: "https://hub.ironclaw.com/web.capabilities.json".to_string(),
                        size_bytes: 1,
                        sha256: "b".repeat(64),
                    },
                })
                .expect("tool summary"),
            ],
            skills: vec![skill],
        },
    );

    let rendered = render_reborn_ironhub_response("search", &response);
    assert!(rendered.contains("- tool web 0.1.0"));
    assert!(rendered.contains("- skill reviewer 0.2.0"));
}

#[test]
fn artifact_digest_binds_both_tool_artifacts() {
    let tool = IronHubToolEntry {
        name: "web".to_string(),
        crate_name: "web-tool".to_string(),
        version: "0.1.0".to_string(),
        description: String::new(),
        provenance: IronHubProvenance::Official,
        wasm: IronHubArtifact {
            url: "https://hub.ironclaw.com/web.wasm".to_string(),
            size_bytes: 1,
            sha256: "a".repeat(64),
        },
        capabilities: IronHubArtifact {
            url: "https://hub.ironclaw.com/web.capabilities.json".to_string(),
            size_bytes: 1,
            sha256: "b".repeat(64),
        },
    };
    assert_eq!(
        tool_artifact_digest(&tool),
        sha256_digest_token(format!("{}:{}", "a".repeat(64), "b".repeat(64)).as_bytes())
    );
}

#[test]
fn artifact_url_rejects_internal_hosts_even_when_extra() {
    assert!(host_is_disallowed_target("localhost"));
    assert!(host_is_disallowed_target("10.0.0.1"));
    assert!(host_is_disallowed_target("service.internal"));
}

#[tokio::test]
async fn search_rejects_untrusted_signed_manifest_from_runtime_egress() {
    let dir = tempfile::tempdir().expect("tempdir");
    let manifest_url = "https://hub.ironclaw.com/tests/reject/manifest.json";
    let egress = Arc::new(RecordingIronHubEgress::new([(
        manifest_url,
        b"not signed".to_vec(),
    )]));
    let service = ironhub_service(dir.path().join("local-dev"), egress, manifest_url).await;

    let error = service
        .execute(super::model::IronHubCommand::Search {
            query: String::new(),
        })
        .await
        .expect_err("bad signed manifest should be rejected");

    assert!(
        error
            .to_string()
            .contains("signed manifest verification failed"),
        "{error}"
    );
}

#[tokio::test]
async fn install_rejects_artifact_sha256_mismatch_before_reborn_write() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("local-dev");
    let manifest_url = "https://hub.ironclaw.com/tests/mismatch/manifest.json";
    let skill_url = "https://hub.ironclaw.com/tests/mismatch/SKILL.md";
    let manifest = signed_manifest(skill_manifest_json(
        "checksum-skill",
        "2026-01-01T00:00:00Z",
        skill_url,
        &sha256_hex(b"expected skill"),
        IronHubProvenance::Official,
    ));
    let egress = Arc::new(RecordingIronHubEgress::new([
        (manifest_url, manifest),
        (skill_url, b"corrupted skill".to_vec()),
    ]));
    let service = ironhub_service(root.clone(), egress, manifest_url).await;

    let error = service
        .execute(super::model::IronHubCommand::Install {
            name: "checksum-skill".to_string(),
            options: IronHubInstallOptions {
                kind: Some(IronHubEntryKind::Skill),
                ..IronHubInstallOptions::default()
            },
        })
        .await
        .expect_err("checksum mismatch should fail before installing");

    assert!(error.to_string().contains("checksum mismatch"), "{error}");
    assert!(
        !root.join("skills/checksum-skill/SKILL.md").exists(),
        "corrupted skill should not be materialized"
    );
}

#[tokio::test]
async fn install_skill_and_tool_materialize_into_reborn_management() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("local-dev");
    let manifest_url = "https://hub.ironclaw.com/tests/install/manifest.json";
    let skill_url = "https://hub.ironclaw.com/tests/install/SKILL.md";
    let wasm_url = "https://hub.ironclaw.com/tests/install/tool.wasm";
    let capabilities_url = "https://hub.ironclaw.com/tests/install/capabilities.json";
    let skill_bytes = b"# installed skill\n";
    let wasm_bytes = b"\0asm";
    let capabilities_bytes = br#"{"capabilities":[]}"#;
    let manifest = signed_manifest(mixed_manifest_json(MixedManifestFixture {
        skill_name: "installed-skill",
        tool_name: "installed-tool",
        generated_at: "2026-01-02T00:00:00Z",
        skill_url,
        skill_sha: &sha256_hex(skill_bytes),
        wasm_url,
        wasm_sha: &sha256_hex(wasm_bytes),
        capabilities_url,
        capabilities_sha: &sha256_hex(capabilities_bytes),
    }));
    let egress = Arc::new(RecordingIronHubEgress::new([
        (manifest_url, manifest),
        (skill_url, skill_bytes.to_vec()),
        (wasm_url, wasm_bytes.to_vec()),
        (capabilities_url, capabilities_bytes.to_vec()),
    ]));
    let service = ironhub_service(root.clone(), egress, manifest_url).await;

    let skill = service
        .execute(super::model::IronHubCommand::Install {
            name: "installed-skill".to_string(),
            options: IronHubInstallOptions {
                kind: Some(IronHubEntryKind::Skill),
                ..IronHubInstallOptions::default()
            },
        })
        .await
        .expect("skill install succeeds");
    assert_eq!(skill.phase, LifecyclePhase::Installed);
    assert!(root.join("skills/installed-skill/SKILL.md").exists());

    let tool = service
        .execute(super::model::IronHubCommand::Install {
            name: "installed-tool".to_string(),
            options: IronHubInstallOptions {
                kind: Some(IronHubEntryKind::Tool),
                ..IronHubInstallOptions::default()
            },
        })
        .await
        .expect("tool install succeeds");
    assert_eq!(tool.phase, LifecyclePhase::Installed);
    assert!(
        root.join("system/extensions/installed-tool/manifest.toml")
            .exists()
    );
}

#[tokio::test]
async fn install_resolves_skill_from_private_manifest_url() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("local-dev");
    let public_manifest_url = "https://hub.ironclaw.com/tests/private/public-manifest.json";
    let private_manifest_url =
        "https://hub.ironclaw.com/tests/private/org-manifest.json?token=test-capability-token";
    let skill_url = "https://hub.ironclaw.com/tests/private/SKILL.md";
    let skill_bytes = b"# private skill\n";
    let private_manifest = signed_manifest(skill_manifest_json(
        "private-skill",
        "2026-01-04T00:00:00Z",
        skill_url,
        &sha256_hex(skill_bytes),
        IronHubProvenance::Private,
    ));
    let egress = Arc::new(RecordingIronHubEgress::new([
        (
            public_manifest_url,
            signed_manifest(empty_manifest_json("2026-01-04T00:00:00Z")),
        ),
        (private_manifest_url, private_manifest),
        (skill_url, skill_bytes.to_vec()),
    ]));
    let service = ironhub_service(root.clone(), egress, public_manifest_url).await;

    let installed = service
        .execute(super::model::IronHubCommand::Install {
            name: "private-skill".to_string(),
            options: IronHubInstallOptions {
                kind: Some(IronHubEntryKind::Skill),
                private_manifest_url: Some(private_manifest_url.to_string()),
                ..IronHubInstallOptions::default()
            },
        })
        .await
        .expect("install resolves the skill from the private manifest");

    assert_eq!(installed.phase, LifecyclePhase::Installed);
    assert!(root.join("skills/private-skill/SKILL.md").exists());
}

#[tokio::test]
async fn fetch_manifest_uses_runtime_egress_host_policy() {
    let dir = tempfile::tempdir().expect("tempdir");
    let manifest_url = "https://hub.ironclaw.com/tests/policy/manifest.json";
    let egress = Arc::new(RecordingIronHubEgress::new([(
        manifest_url,
        signed_manifest(empty_manifest_json("2026-01-03T00:00:00Z")),
    )]));
    let service = ironhub_service(
        dir.path().join("local-dev"),
        egress.clone() as Arc<dyn RuntimeHttpEgress>,
        manifest_url,
    )
    .await;

    service
        .execute(super::model::IronHubCommand::Search {
            query: String::new(),
        })
        .await
        .expect("manifest fetch succeeds");

    let request = egress.single_request();
    assert_eq!(request.runtime, RuntimeKind::FirstParty);
    assert_eq!(request.url, manifest_url);
    assert_eq!(
        request.capability_id,
        CapabilityId::new(super::model::IRONHUB_SEARCH_CAPABILITY_ID).expect("capability id")
    );
    assert_eq!(
        request.response_body_limit,
        Some(super::model::MAX_SIGNED_MANIFEST_BYTES)
    );
    assert_eq!(request.network_policy.allowed_targets.len(), 1);
    let target = &request.network_policy.allowed_targets[0];
    assert_eq!(target.scheme, Some(NetworkScheme::Https));
    assert_eq!(target.host_pattern, "hub.ironclaw.com");
    assert!(request.network_policy.deny_private_ip_ranges);
    assert_eq!(
        request.network_policy.max_egress_bytes,
        Some(super::model::MAX_SIGNED_MANIFEST_BYTES)
    );
}

#[tokio::test]
async fn concurrent_manifest_cache_miss_fetches_once() {
    let dir = tempfile::tempdir().expect("tempdir");
    let manifest_url = "https://hub.ironclaw.com/tests/singleflight/manifest.json";
    let egress = Arc::new(
        RecordingIronHubEgress::new([(
            manifest_url,
            signed_manifest(empty_manifest_json("2026-01-04T00:00:00Z")),
        )])
        .with_delay(Duration::from_millis(50)),
    );
    let service = Arc::new(
        ironhub_service(
            dir.path().join("local-dev"),
            egress.clone() as Arc<dyn RuntimeHttpEgress>,
            manifest_url,
        )
        .await,
    );

    let first = {
        let service = Arc::clone(&service);
        tokio::spawn(async move {
            service
                .execute(super::model::IronHubCommand::Search {
                    query: String::new(),
                })
                .await
        })
    };
    let second = {
        let service = Arc::clone(&service);
        tokio::spawn(async move {
            service
                .execute(super::model::IronHubCommand::Search {
                    query: String::new(),
                })
                .await
        })
    };

    first.await.expect("first task").expect("first search");
    second.await.expect("second task").expect("second search");
    assert_eq!(egress.request_count(), 1);
}

async fn ironhub_service(
    local_dev_root: std::path::PathBuf,
    egress: Arc<dyn RuntimeHttpEgress>,
    manifest_url: &str,
) -> IronHubService {
    let services = build_reborn_services(RebornBuildInput::local_dev(
        "ironhub-test-owner",
        local_dev_root,
    ))
    .await
    .expect("local-dev services build");
    let local_runtime = services.local_runtime.expect("local runtime substrate");
    IronHubService::new_with_runtime_egress(
        Arc::clone(&local_runtime.skill_management),
        local_runtime
            .extension_management
            .as_ref()
            .expect("extension management")
            .clone(),
        egress,
        CapabilityId::new(super::model::IRONHUB_SEARCH_CAPABILITY_ID).expect("capability id"),
        ResourceScope::local_default(
            UserId::new("ironhub-test-user").expect("user"),
            InvocationId::new(),
        )
        .expect("scope"),
    )
    .with_manifest_url(manifest_url)
    .with_manifest_verify_keys(test_manifest_verify_keys())
}

fn signed_manifest(manifest_json: String) -> Vec<u8> {
    let signing_key = test_signing_key();
    let signature = signing_key.sign(manifest_json.as_bytes());
    serde_json::json!({
        "v": 1,
        "key_id": "ironhub-test-key",
        "manifest_b64": URL_SAFE_NO_PAD.encode(manifest_json.as_bytes()),
        "sig": URL_SAFE_NO_PAD.encode(signature.to_bytes()),
    })
    .to_string()
    .into_bytes()
}

fn test_manifest_verify_keys() -> &'static [(&'static str, &'static str)] {
    let signing_key = test_signing_key();
    let verify_key = hex::encode(signing_key.verifying_key().to_bytes());
    let verify_key: &'static str = Box::leak(verify_key.into_boxed_str());
    Box::leak(vec![("ironhub-test-key", verify_key)].into_boxed_slice())
}

fn test_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[7_u8; 32])
}

fn empty_manifest_json(generated_at: &str) -> String {
    format!(
        r#"{{"version":"1","generated_at":"{generated_at}","release_tag":"test","repo":"nearai/ironhub","tools":[],"skills":[]}}"#
    )
}

fn skill_manifest_json(
    name: &str,
    generated_at: &str,
    skill_url: &str,
    skill_sha: &str,
    provenance: IronHubProvenance,
) -> String {
    format!(
        r#"{{"version":"1","generated_at":"{generated_at}","release_tag":"test","repo":"nearai/ironhub","tools":[],"skills":[{{"name":"{name}","version":"0.1.0","description":"test skill","provenance":"{}","skill_md":{{"url":"{skill_url}","size_bytes":1048576,"sha256":"{skill_sha}"}}}}]}}"#,
        provenance.as_wire()
    )
}

struct MixedManifestFixture<'a> {
    skill_name: &'a str,
    tool_name: &'a str,
    generated_at: &'a str,
    skill_url: &'a str,
    skill_sha: &'a str,
    wasm_url: &'a str,
    wasm_sha: &'a str,
    capabilities_url: &'a str,
    capabilities_sha: &'a str,
}

fn mixed_manifest_json(fixture: MixedManifestFixture<'_>) -> String {
    let MixedManifestFixture {
        skill_name,
        tool_name,
        generated_at,
        skill_url,
        skill_sha,
        wasm_url,
        wasm_sha,
        capabilities_url,
        capabilities_sha,
    } = fixture;
    format!(
        r#"{{"version":"1","generated_at":"{generated_at}","release_tag":"test","repo":"nearai/ironhub","tools":[{{"name":"{tool_name}","crate_name":"{tool_name}","version":"0.1.0","description":"test tool","provenance":"official","wasm":{{"url":"{wasm_url}","size_bytes":1048576,"sha256":"{wasm_sha}"}},"capabilities":{{"url":"{capabilities_url}","size_bytes":1048576,"sha256":"{capabilities_sha}"}}}}],"skills":[{{"name":"{skill_name}","version":"0.1.0","description":"test skill","provenance":"official","skill_md":{{"url":"{skill_url}","size_bytes":1048576,"sha256":"{skill_sha}"}}}}]}}"#
    )
}

#[derive(Debug, Clone)]
struct RecordedEgressRequest {
    runtime: RuntimeKind,
    capability_id: CapabilityId,
    url: String,
    response_body_limit: Option<u64>,
    network_policy: ironclaw_host_api::NetworkPolicy,
}

struct RecordingIronHubEgress {
    responses: Mutex<HashMap<String, VecDeque<Vec<u8>>>>,
    requests: Mutex<Vec<RecordedEgressRequest>>,
    delay: Option<Duration>,
}

impl RecordingIronHubEgress {
    fn new<const N: usize>(responses: [(&str, Vec<u8>); N]) -> Self {
        let responses = responses
            .into_iter()
            .map(|(url, body)| (url.to_string(), VecDeque::from([body])))
            .collect();
        Self {
            responses: Mutex::new(responses),
            requests: Mutex::new(Vec::new()),
            delay: None,
        }
    }

    fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = Some(delay);
        self
    }

    fn single_request(&self) -> RecordedEgressRequest {
        let requests = self.requests.lock().expect("requests lock");
        assert_eq!(requests.len(), 1);
        requests[0].clone()
    }

    fn request_count(&self) -> usize {
        self.requests.lock().expect("requests lock").len()
    }
}

#[async_trait]
impl RuntimeHttpEgress for RecordingIronHubEgress {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        self.requests
            .lock()
            .expect("requests lock")
            .push(RecordedEgressRequest {
                runtime: request.runtime,
                capability_id: request.capability_id.clone(),
                url: request.url.clone(),
                response_body_limit: request.response_body_limit,
                network_policy: request.network_policy.clone(),
            });
        if let Some(delay) = self.delay {
            tokio::time::sleep(delay).await;
        }
        let body = self
            .responses
            .lock()
            .expect("responses lock")
            .get_mut(&request.url)
            .and_then(VecDeque::pop_front)
            .ok_or_else(|| RuntimeHttpEgressError::Request {
                reason: format!("unexpected IronHub test URL {}", request.url),
                request_bytes: 0,
                response_bytes: 0,
            })?;
        Ok(RuntimeHttpEgressResponse {
            status: 200,
            headers: Vec::new(),
            body,
            saved_body: None,
            request_bytes: 0,
            response_bytes: 0,
            redaction_applied: false,
        })
    }
}
