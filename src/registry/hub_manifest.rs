use serde::{Deserialize, Serialize};

pub const DEFAULT_HUB_MANIFEST_URL: &str = "https://hub.ironclaw.com/api/catalog/manifest.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    #[default]
    #[serde(alias = "repo")]
    Official,
    Trusted,
    Verified,
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
    fn provenance_defaults_to_official_when_field_absent() {
        let manifest: HubManifest = serde_json::from_str(SAMPLE_MANIFEST).expect("valid manifest");
        assert_eq!(manifest.tools[0].provenance, Provenance::Official);
        assert_eq!(manifest.skills[0].provenance, Provenance::Official);
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
