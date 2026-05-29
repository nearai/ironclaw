use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};

pub const DEFAULT_HUB_MANIFEST_URL: &str = "https://hub.ironclaw.com/api/catalog/manifest.json";

// Ed25519 PUBLIC keys trusted to verify the catalog manifest signature, by key_id.
// PUBLIC KEYS ONLY. The private signing key lives off the catalog host (IronHub
// IRONHUB_MANIFEST_SIGNING_KEY) and must never appear in this repo or binary.
// Embedding the public half is deliberate: changing which key the agent trusts
// requires changing this source, not compromising the catalog host.
pub const MANIFEST_VERIFY_KEYS: &[(&str, &str)] = &[(
    "5895a21abea89672",
    "f64d2d3a3228b16ca59450364d26b278071a1a425544f242504033341d8459bd",
)];

#[derive(Debug, Deserialize)]
struct SignedManifestEnvelope {
    v: u8,
    key_id: String,
    manifest_b64: String,
    sig: String,
}

fn verifying_key_from_hex(hex: &str) -> Result<VerifyingKey, String> {
    if hex.len() != 64 {
        return Err("verify key must be 64 hex chars".to_string());
    }
    let mut raw = [0u8; 32];
    for (i, byte) in raw.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
            .map_err(|_| "verify key is not valid hex".to_string())?;
    }
    VerifyingKey::from_bytes(&raw).map_err(|e| format!("invalid ed25519 verify key: {e}"))
}

/// Verifies a signed-manifest envelope and returns the exact inner manifest bytes
/// the signature covers. Fail-closed: any decode, key-lookup, or signature failure
/// returns Err and never yields partial bytes.
pub fn verify_signed_manifest(
    envelope_bytes: &[u8],
    keys: &[(&str, &str)],
) -> Result<Vec<u8>, String> {
    let env: SignedManifestEnvelope = serde_json::from_slice(envelope_bytes)
        .map_err(|e| format!("signed-manifest envelope parse failed: {e}"))?;
    if env.v != 1 {
        return Err(format!("unsupported signed-manifest version {}", env.v));
    }
    let key_hex = keys
        .iter()
        .find(|(id, _)| *id == env.key_id)
        .map(|(_, hex)| *hex)
        .ok_or_else(|| format!("unknown manifest signing key_id '{}'", env.key_id))?;
    let verifying_key = verifying_key_from_hex(key_hex)?;
    let manifest_bytes = URL_SAFE_NO_PAD
        .decode(env.manifest_b64.as_bytes())
        .map_err(|e| format!("manifest_b64 decode failed: {e}"))?;
    let sig_bytes = URL_SAFE_NO_PAD
        .decode(env.sig.as_bytes())
        .map_err(|e| format!("signature decode failed: {e}"))?;
    let signature =
        Signature::from_slice(&sig_bytes).map_err(|e| format!("signature malformed: {e}"))?;
    verifying_key
        .verify_strict(&manifest_bytes, &signature)
        .map_err(|_| "manifest signature verification failed".to_string())?;
    Ok(manifest_bytes)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    #[serde(alias = "repo")]
    Official,
    Trusted,
    Verified,
    #[default]
    #[serde(alias = "community")]
    New,
}

impl Provenance {
    pub fn as_wire(&self) -> &'static str {
        match self {
            Provenance::Official => "official",
            Provenance::Trusted => "trusted",
            Provenance::Verified => "verified",
            Provenance::New => "new",
        }
    }

    pub fn is_community_unverified(&self) -> bool {
        matches!(self, Provenance::New)
    }

    pub fn trust_label(&self) -> &'static str {
        match self {
            Provenance::Official => "NEAR-vetted (official)",
            Provenance::Trusted => "community, trusted publisher",
            Provenance::Verified => "community, verified publisher",
            Provenance::New => "UNVERIFIED community (new author)",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HubManifest {
    pub version: String,
    pub generated_at: String,
    pub release_tag: String,
    pub repo: String,
    #[serde(default)]
    pub tools: Vec<HubToolEntry>,
    #[serde(default)]
    pub skills: Vec<HubSkillEntry>,
}

impl HubManifest {
    pub fn find_tool(&self, name: &str) -> Option<&HubToolEntry> {
        self.tools.iter().find(|t| t.name == name)
    }

    pub fn find_skill(&self, name: &str) -> Option<&HubSkillEntry> {
        self.skills.iter().find(|s| s.name == name)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HubToolEntry {
    pub name: String,
    pub crate_name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub provenance: Provenance,
    pub wasm: HubArtifact,
    pub capabilities: HubArtifact,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HubSkillEntry {
    pub name: String,
    #[serde(default)]
    pub trunk: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub provenance: Provenance,
    pub skill_md: HubArtifact,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HubArtifact {
    pub url: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shared cross-language vector: a fixed manifest signed with a throwaway test
    // Ed25519 key. The same (pubkey, manifest_b64, sig) tuple is asserted in the
    // IronHub signer's tests. All public; the test private key was discarded.
    const VEC_KEY_ID: &str = "test-vector";
    const VEC_PUBKEY_HEX: &str = "ca46572f4dcd485599cdf95442934a3e3c86e2cae766a85fbffc8d6540959928";
    const VEC_MANIFEST_B64: &str = "eyJ2ZXJzaW9uIjoiMSIsImdlbmVyYXRlZF9hdCI6IjIwMjYtMDEtMDFUMDA6MDA6MDBaIiwicmVsZWFzZV90YWciOiJ0ZXN0IiwicmVwbyI6Im5lYXJhaS9pcm9uaHViIiwidG9vbHMiOltdLCJza2lsbHMiOltdfQ";
    const VEC_SIG: &str =
        "KjsUDgi1enj3iTPNQI6gU1Bwxf01hIUItlFvX9PxgWNybPPrJNIV7vFG-G8hJOalFMwFs5zQHrxbtFDZAlgtBg";
    const VEC_MANIFEST_BYTES: &str = r#"{"version":"1","generated_at":"2026-01-01T00:00:00Z","release_tag":"test","repo":"nearai/ironhub","tools":[],"skills":[]}"#;

    fn vec_keys() -> Vec<(&'static str, &'static str)> {
        vec![(VEC_KEY_ID, VEC_PUBKEY_HEX)]
    }

    fn vec_envelope(manifest_b64: &str, sig: &str) -> String {
        format!(r#"{{"v":1,"key_id":"test-vector","manifest_b64":"{manifest_b64}","sig":"{sig}"}}"#)
    }

    #[test]
    fn verify_signed_manifest_accepts_valid_vector() {
        let env = vec_envelope(VEC_MANIFEST_B64, VEC_SIG);
        let bytes =
            verify_signed_manifest(env.as_bytes(), &vec_keys()).expect("valid vector must verify");
        assert_eq!(bytes, VEC_MANIFEST_BYTES.as_bytes());
    }

    #[test]
    fn verify_signed_manifest_rejects_tampered_manifest() {
        let tampered = URL_SAFE_NO_PAD.encode(br#"{"version":"1","tools":[{"name":"evil"}]}"#);
        let env = vec_envelope(&tampered, VEC_SIG);
        assert!(verify_signed_manifest(env.as_bytes(), &vec_keys()).is_err());
    }

    #[test]
    fn verify_signed_manifest_rejects_wrong_key() {
        let wrong = vec![(VEC_KEY_ID, MANIFEST_VERIFY_KEYS[0].1)];
        let env = vec_envelope(VEC_MANIFEST_B64, VEC_SIG);
        assert!(verify_signed_manifest(env.as_bytes(), &wrong).is_err());
    }

    #[test]
    fn verify_signed_manifest_rejects_unknown_key_id() {
        let env = format!(
            r#"{{"v":1,"key_id":"nope","manifest_b64":"{VEC_MANIFEST_B64}","sig":"{VEC_SIG}"}}"#
        );
        assert!(verify_signed_manifest(env.as_bytes(), &vec_keys()).is_err());
    }

    #[test]
    fn embedded_manifest_verify_keys_are_valid_public_keys() {
        for (key_id, hex) in MANIFEST_VERIFY_KEYS {
            assert_eq!(hex.len(), 64, "key {key_id} must be 64 hex chars");
            assert!(
                verifying_key_from_hex(hex).is_ok(),
                "embedded key {key_id} must decode to a valid ed25519 public key"
            );
        }
    }

    const SAMPLE_MANIFEST: &str = r#"{
        "version": "1",
        "generated_at": "2026-05-12T23:43:46Z",
        "release_tag": "release-2026-05-12-24",
        "repo": "nearai/ironhub",
        "tools": [
            {
                "name": "clickup",
                "crate_name": "clickup-tool",
                "version": "0.1.0",
                "description": "ClickUp integration",
                "wasm": {
                    "url": "https://github.com/nearai/ironhub/releases/download/release-2026-05-12-24/clickup.wasm",
                    "size_bytes": 433139,
                    "sha256": "f96f9f24c379a9bcf714e3fb7692a712b1ffd8432884af0a2120f1ad1bb8c619"
                },
                "capabilities": {
                    "url": "https://github.com/nearai/ironhub/releases/download/release-2026-05-12-24/clickup.capabilities.json",
                    "size_bytes": 3287,
                    "sha256": "1815aa5019cf4b329ee3269a5a3bbd301f690c4d9505c3fd4e5062983cedc4ef"
                }
            }
        ],
        "skills": [
            {
                "name": "microsoft-365-workflow",
                "trunk": "microsoft-365",
                "version": "1.0.0",
                "description": "Microsoft 365 business workflow patterns",
                "skill_md": {
                    "url": "https://github.com/nearai/ironhub/releases/download/release-2026-05-12-24/microsoft-365-workflow.SKILL.md",
                    "size_bytes": 14000,
                    "sha256": "a1b2c3d4e5f6789012345678901234567890123456789012345678901234abcd"
                }
            }
        ]
    }"#;

    #[test]
    fn parses_sample_manifest() {
        let manifest: HubManifest = serde_json::from_str(SAMPLE_MANIFEST).expect("valid manifest");
        assert_eq!(manifest.version, "1");
        assert_eq!(manifest.release_tag, "release-2026-05-12-24");
        assert_eq!(manifest.repo, "nearai/ironhub");
        assert_eq!(manifest.tools.len(), 1);
        assert_eq!(manifest.skills.len(), 1);
    }

    #[test]
    fn find_tool_returns_entry_by_name() {
        let manifest: HubManifest = serde_json::from_str(SAMPLE_MANIFEST).expect("valid manifest");
        let tool = manifest.find_tool("clickup").expect("clickup present");
        assert_eq!(tool.crate_name, "clickup-tool");
        assert_eq!(tool.wasm.size_bytes, 433139);
        assert!(manifest.find_tool("nonexistent").is_none());
    }

    #[test]
    fn find_skill_returns_entry_by_name() {
        let manifest: HubManifest = serde_json::from_str(SAMPLE_MANIFEST).expect("valid manifest");
        let skill = manifest
            .find_skill("microsoft-365-workflow")
            .expect("skill present");
        assert_eq!(skill.trunk, "microsoft-365");
        assert!(manifest.find_skill("nonexistent").is_none());
    }

    #[test]
    fn default_manifest_url_is_https_hub_endpoint() {
        assert_eq!(
            DEFAULT_HUB_MANIFEST_URL,
            "https://hub.ironclaw.com/api/catalog/manifest.json"
        );
    }

    #[test]
    fn provenance_defaults_to_new_when_field_absent() {
        let manifest: HubManifest = serde_json::from_str(SAMPLE_MANIFEST).expect("valid manifest");
        assert_eq!(manifest.tools[0].provenance, Provenance::New);
        assert_eq!(manifest.skills[0].provenance, Provenance::New);
    }

    #[test]
    fn provenance_parses_each_tier_and_aliases() {
        let cases = [
            (r#""official""#, Provenance::Official),
            (r#""repo""#, Provenance::Official),
            (r#""trusted""#, Provenance::Trusted),
            (r#""verified""#, Provenance::Verified),
            (r#""new""#, Provenance::New),
            (r#""community""#, Provenance::New),
        ];
        for (raw, expected) in cases {
            let got: Provenance = serde_json::from_str(raw).expect("valid provenance");
            assert_eq!(got, expected, "input {raw}");
        }
    }

    #[test]
    fn provenance_rejects_unknown_string() {
        assert!(serde_json::from_str::<Provenance>(r#""banned""#).is_err());
        assert!(serde_json::from_str::<Provenance>(r#""whatever""#).is_err());
    }

    #[test]
    fn provenance_wire_round_trips() {
        for p in [
            Provenance::Official,
            Provenance::Trusted,
            Provenance::Verified,
            Provenance::New,
        ] {
            let json = serde_json::to_string(&p).expect("serialize");
            assert_eq!(json, format!("\"{}\"", p.as_wire()));
            let back: Provenance = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, p);
        }
        assert!(Provenance::New.is_community_unverified());
        assert!(!Provenance::Official.is_community_unverified());
    }

    #[test]
    fn manifest_with_no_tools_or_skills_parses() {
        let raw = r#"{
            "version": "1",
            "generated_at": "2026-05-12T23:43:46Z",
            "release_tag": "release-2026-05-12-24",
            "repo": "nearai/ironhub"
        }"#;
        let manifest: HubManifest = serde_json::from_str(raw).expect("valid manifest");
        assert!(manifest.tools.is_empty());
        assert!(manifest.skills.is_empty());
    }
}
