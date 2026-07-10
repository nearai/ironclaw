//! Complete registry of known v1 database and home-directory state.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use ironclaw_common::hashing::sha256_hex;

use crate::manifest::{Disposition, InventoryEntry, InventorySourceKind};
use crate::report::Domain;

#[derive(Debug, Clone, Copy)]
struct DispositionRule {
    name: &'static str,
    domain: Domain,
    disposition: Disposition,
}

const TABLE_RULES: &[DispositionRule] = &[
    DispositionRule {
        name: "_migrations",
        domain: Domain::SchemaMetadata,
        disposition: Disposition::IntentionallyReset,
    },
    DispositionRule {
        name: "refinery_schema_history",
        domain: Domain::SchemaMetadata,
        disposition: Disposition::IntentionallyReset,
    },
    DispositionRule {
        name: "conversations",
        domain: Domain::Thread,
        disposition: Disposition::Imported,
    },
    DispositionRule {
        name: "conversation_messages",
        domain: Domain::Message,
        disposition: Disposition::Imported,
    },
    DispositionRule {
        name: "agent_jobs",
        domain: Domain::Job,
        disposition: Disposition::ArchiveOnly,
    },
    DispositionRule {
        name: "job_actions",
        domain: Domain::Job,
        disposition: Disposition::ArchiveOnly,
    },
    DispositionRule {
        name: "job_events",
        domain: Domain::Job,
        disposition: Disposition::ArchiveOnly,
    },
    DispositionRule {
        name: "dynamic_tools",
        domain: Domain::Extension,
        disposition: Disposition::ArchiveOnly,
    },
    DispositionRule {
        name: "llm_calls",
        domain: Domain::OperationalState,
        disposition: Disposition::ArchiveOnly,
    },
    DispositionRule {
        name: "estimation_snapshots",
        domain: Domain::OperationalState,
        disposition: Disposition::ArchiveOnly,
    },
    DispositionRule {
        name: "repair_attempts",
        domain: Domain::OperationalState,
        disposition: Disposition::ArchiveOnly,
    },
    DispositionRule {
        name: "memory_documents",
        domain: Domain::Memory,
        disposition: Disposition::Imported,
    },
    DispositionRule {
        name: "memory_chunks",
        domain: Domain::Memory,
        disposition: Disposition::DerivedRebuilt,
    },
    DispositionRule {
        name: "memory_document_versions",
        domain: Domain::Memory,
        disposition: Disposition::ArchiveOnly,
    },
    DispositionRule {
        name: "heartbeat_state",
        domain: Domain::Heartbeat,
        disposition: Disposition::SemanticallyConverted,
    },
    DispositionRule {
        name: "secrets",
        domain: Domain::Secret,
        disposition: Disposition::Imported,
    },
    DispositionRule {
        name: "wasm_tools",
        domain: Domain::Extension,
        disposition: Disposition::RequiresReinstall,
    },
    DispositionRule {
        name: "wasm_channels",
        domain: Domain::Extension,
        disposition: Disposition::RequiresReinstall,
    },
    DispositionRule {
        name: "tool_capabilities",
        domain: Domain::Extension,
        disposition: Disposition::RequiresReinstall,
    },
    DispositionRule {
        name: "leak_detection_patterns",
        domain: Domain::SecurityAudit,
        disposition: Disposition::ArchiveOnly,
    },
    DispositionRule {
        name: "tool_rate_limit_state",
        domain: Domain::OperationalState,
        disposition: Disposition::IntentionallyReset,
    },
    DispositionRule {
        name: "secret_usage_log",
        domain: Domain::SecurityAudit,
        disposition: Disposition::ArchiveOnly,
    },
    DispositionRule {
        name: "leak_detection_events",
        domain: Domain::SecurityAudit,
        disposition: Disposition::ArchiveOnly,
    },
    DispositionRule {
        name: "tool_failures",
        domain: Domain::OperationalState,
        disposition: Disposition::ArchiveOnly,
    },
    DispositionRule {
        name: "routines",
        domain: Domain::Routine,
        disposition: Disposition::SemanticallyConverted,
    },
    DispositionRule {
        name: "routine_runs",
        domain: Domain::Routine,
        disposition: Disposition::ArchiveOnly,
    },
    DispositionRule {
        name: "settings",
        domain: Domain::Setting,
        disposition: Disposition::SemanticallyConverted,
    },
    DispositionRule {
        name: "users",
        domain: Domain::User,
        disposition: Disposition::Imported,
    },
    DispositionRule {
        name: "api_tokens",
        domain: Domain::ApiToken,
        disposition: Disposition::RequiresReauth,
    },
    DispositionRule {
        name: "user_identities",
        domain: Domain::Identity,
        disposition: Disposition::Imported,
    },
    DispositionRule {
        name: "channel_identities",
        domain: Domain::Identity,
        disposition: Disposition::SemanticallyConverted,
    },
    DispositionRule {
        name: "pairing_requests",
        domain: Domain::Pairing,
        disposition: Disposition::IntentionallyReset,
    },
    DispositionRule {
        name: "claude_code_events",
        domain: Domain::OperationalState,
        disposition: Disposition::ArchiveOnly,
    },
    DispositionRule {
        name: "root_filesystem_entries",
        domain: Domain::WorkspaceFile,
        disposition: Disposition::SemanticallyConverted,
    },
    DispositionRule {
        name: "root_filesystem_events",
        domain: Domain::OperationalState,
        disposition: Disposition::ArchiveOnly,
    },
    DispositionRule {
        name: "root_filesystem_index_specs",
        domain: Domain::OperationalState,
        disposition: Disposition::DerivedRebuilt,
    },
    DispositionRule {
        name: "root_filesystem_sequences",
        domain: Domain::OperationalState,
        disposition: Disposition::IntentionallyReset,
    },
    DispositionRule {
        name: "hooks_predicate_invocations",
        domain: Domain::OperationalState,
        disposition: Disposition::ArchiveOnly,
    },
    DispositionRule {
        name: "hooks_predicate_values",
        domain: Domain::OperationalState,
        disposition: Disposition::ArchiveOnly,
    },
];

const FILE_RULES: &[DispositionRule] = &[
    DispositionRule {
        name: ".env",
        domain: Domain::Setting,
        disposition: Disposition::SemanticallyConverted,
    },
    DispositionRule {
        name: "settings.json",
        domain: Domain::Setting,
        disposition: Disposition::SemanticallyConverted,
    },
    DispositionRule {
        name: "config.toml",
        domain: Domain::Setting,
        disposition: Disposition::SemanticallyConverted,
    },
    DispositionRule {
        name: "providers.json",
        domain: Domain::Provider,
        disposition: Disposition::SemanticallyConverted,
    },
    DispositionRule {
        name: "session.json",
        domain: Domain::ApiToken,
        disposition: Disposition::RequiresReauth,
    },
    DispositionRule {
        name: "mcp-servers.json",
        domain: Domain::Extension,
        disposition: Disposition::RequiresReinstall,
    },
    DispositionRule {
        name: "acp-agents.json",
        domain: Domain::Extension,
        disposition: Disposition::RequiresReinstall,
    },
    DispositionRule {
        name: "history",
        domain: Domain::OperationalState,
        disposition: Disposition::IntentionallyReset,
    },
];

const DIRECTORY_RULES: &[DispositionRule] = &[
    DispositionRule {
        name: "profiles",
        domain: Domain::Setting,
        disposition: Disposition::SemanticallyConverted,
    },
    DispositionRule {
        name: "skills",
        domain: Domain::Skill,
        disposition: Disposition::SemanticallyConverted,
    },
    DispositionRule {
        name: "installed_skills",
        domain: Domain::Skill,
        disposition: Disposition::RequiresReinstall,
    },
    DispositionRule {
        name: "tools",
        domain: Domain::Extension,
        disposition: Disposition::RequiresReinstall,
    },
    DispositionRule {
        name: "channels",
        domain: Domain::Extension,
        disposition: Disposition::RequiresReinstall,
    },
    DispositionRule {
        name: "projects",
        domain: Domain::Project,
        disposition: Disposition::SemanticallyConverted,
    },
    DispositionRule {
        name: "logs",
        domain: Domain::OperationalState,
        disposition: Disposition::IntentionallyReset,
    },
];

#[derive(Debug, Clone)]
pub(crate) struct RawTableInventory {
    pub name: String,
    pub count: u64,
    pub checksum: String,
}

pub(crate) fn build_table_inventory(raw: Vec<RawTableInventory>) -> Vec<InventoryEntry> {
    let mut by_name: BTreeMap<_, _> = raw
        .into_iter()
        .map(|item| (item.name.clone(), item))
        .collect();
    let mut out = Vec::with_capacity(TABLE_RULES.len() + by_name.len());
    for rule in TABLE_RULES {
        let raw = by_name.remove(rule.name);
        out.push(InventoryEntry {
            source_kind: InventorySourceKind::Table,
            source_name: rule.name.to_string(),
            domain: rule.domain,
            disposition: rule.disposition,
            count: raw.as_ref().map_or(0, |item| item.count),
            checksum: raw.map_or_else(
                || sha256_hex(format!("missing:{}", rule.name).as_bytes()),
                |item| item.checksum,
            ),
            blocker: None,
            warning: None,
        });
    }
    for (_, raw) in by_name {
        if let Some(rule) = dynamic_table_rule(&raw.name) {
            out.push(InventoryEntry {
                source_kind: InventorySourceKind::Table,
                source_name: raw.name,
                domain: rule.domain,
                disposition: rule.disposition,
                count: raw.count,
                checksum: raw.checksum,
                blocker: None,
                warning: None,
            });
            continue;
        }
        out.push(InventoryEntry {
            source_kind: InventorySourceKind::Table,
            source_name: raw.name,
            domain: Domain::Unknown,
            disposition: Disposition::UnsupportedUnknown,
            count: raw.count,
            checksum: raw.checksum,
            blocker: Some(
                "unknown v1 table requires an explicit migration disposition".to_string(),
            ),
            warning: None,
        });
    }
    out
}

fn dynamic_table_rule(name: &str) -> Option<DispositionRule> {
    if name == "memory_chunks_fts"
        || matches!(
            name.strip_prefix("memory_chunks_fts_"),
            Some("config" | "content" | "data" | "docsize" | "idx")
        )
    {
        return Some(DispositionRule {
            name: "memory_chunks_fts_shadow",
            domain: Domain::Memory,
            disposition: Disposition::DerivedRebuilt,
        });
    }
    // PostgreSQL installations can have one physical audit partition per
    // month. They share the parent table's archive-only disposition, while an
    // arbitrary unknown table remains a hard blocker.
    if name
        .strip_prefix("secret_usage_log_y")
        .is_some_and(|suffix| suffix.len() == 7 && suffix.as_bytes()[4] == b'm')
    {
        return Some(DispositionRule {
            name: "secret_usage_log_partition",
            domain: Domain::SecurityAudit,
            disposition: Disposition::ArchiveOnly,
        });
    }
    None
}

pub(crate) fn build_home_inventory(
    home: Option<&Path>,
    source_db: Option<&Path>,
    target_db: Option<&Path>,
) -> Vec<InventoryEntry> {
    let mut out = Vec::new();
    let Some(home) = home else { return out };
    let mut excluded_names = BTreeSet::new();
    for path in [source_db, target_db].into_iter().flatten() {
        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            excluded_names.extend([
                name.to_string(),
                format!("{name}-wal"),
                format!("{name}-shm"),
            ]);
        }
        if let Ok(relative) = path.strip_prefix(home)
            && let Some(name) = relative
                .components()
                .next()
                .and_then(|component| component.as_os_str().to_str())
        {
            excluded_names.insert(name.to_string());
        }
    }

    for rule in FILE_RULES {
        out.push(home_entry(
            home.join(rule.name),
            InventorySourceKind::HomeFile,
            *rule,
        ));
    }
    for rule in DIRECTORY_RULES {
        out.push(home_entry(
            home.join(rule.name),
            InventorySourceKind::HomeDirectory,
            *rule,
        ));
    }

    let known: BTreeSet<&str> = FILE_RULES
        .iter()
        .chain(DIRECTORY_RULES)
        .map(|rule| rule.name)
        .collect();
    if let Ok(entries) = std::fs::read_dir(home) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if known.contains(name.as_str()) || excluded_names.contains(&name) {
                continue;
            }
            let kind = entry
                .file_type()
                .ok()
                .map_or(InventorySourceKind::HomeFile, |kind| {
                    if kind.is_dir() {
                        InventorySourceKind::HomeDirectory
                    } else {
                        InventorySourceKind::HomeFile
                    }
                });
            let count = count_path_entries(&entry.path());
            let checksum = checksum_path(&entry.path()).unwrap_or_else(|error| {
                sha256_hex(format!("unreadable-home:{name}:{error}").as_bytes())
            });
            out.push(InventoryEntry {
                source_kind: kind,
                source_name: name.clone(),
                domain: Domain::Unknown,
                disposition: Disposition::UnsupportedUnknown,
                count,
                checksum,
                blocker: Some(
                    "unknown v1 home artifact requires an explicit migration disposition"
                        .to_string(),
                ),
                warning: None,
            });
        }
    }
    out
}

fn home_entry(
    path: PathBuf,
    source_kind: InventorySourceKind,
    rule: DispositionRule,
) -> InventoryEntry {
    let count = count_path_entries(&path);
    let checksum = checksum_path(&path)
        .unwrap_or_else(|error| sha256_hex(format!("unreadable:{}:{error}", rule.name).as_bytes()));
    InventoryEntry {
        source_kind,
        source_name: rule.name.to_string(),
        domain: rule.domain,
        disposition: rule.disposition,
        count,
        checksum,
        blocker: None,
        warning: None,
    }
}

fn checksum_path(path: &Path) -> std::io::Result<String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(sha256_hex(b"missing"));
        }
        Err(error) => return Err(error),
    };
    let mut state = Fnv1a64::new();
    if metadata.file_type().is_symlink() {
        // A symlink destination may itself contain a username, credential, or
        // other operator-private path. Inventory needs only the artifact's
        // shape, not a digest derived from its destination.
        state.update(b"symlink\0");
    } else if metadata.is_file() {
        // Never hash file contents here. Known home files include `.env`,
        // provider configuration, and session state; even a one-way digest of
        // their plaintext would violate the redacted manifest contract.
        state.update(b"file\0");
    } else if metadata.is_dir() {
        state.update(b"directory\0");
        let mut entries = std::fs::read_dir(path)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            state.update(entry.file_name().as_encoded_bytes());
            state.update(b"\0");
            state.update(checksum_path(&entry.path())?.as_bytes());
            state.update(b"\0");
        }
    } else {
        state.update(b"other\0");
    }
    update_metadata_shape(&mut state, &metadata);
    Ok(format!("metadata-fnv1a64:{:016x}", state.finish()))
}

fn update_metadata_shape(state: &mut Fnv1a64, metadata: &std::fs::Metadata) {
    state.update(&metadata.len().to_le_bytes());
    if let Ok(modified) = metadata.modified()
        && let Ok(since_epoch) = modified.duration_since(std::time::UNIX_EPOCH)
    {
        state.update(&since_epoch.as_secs().to_le_bytes());
        state.update(&since_epoch.subsec_nanos().to_le_bytes());
    }
}

struct Fnv1a64(u64);

impl Fnv1a64 {
    const fn new() -> Self {
        Self(0xcbf29ce484222325)
    }

    fn update(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }

    const fn finish(self) -> u64 {
        self.0
    }
}

fn count_path_entries(path: &Path) -> u64 {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return 1;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return 1;
    };
    1 + entries
        .flatten()
        .map(|entry| count_path_entries(&entry.path()))
        .sum::<u64>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_no_duplicate_source_names() {
        let mut names = BTreeSet::new();
        for rule in TABLE_RULES {
            assert!(
                names.insert(rule.name),
                "duplicate table rule: {}",
                rule.name
            );
        }
    }

    #[test]
    fn home_checksum_is_not_derived_from_secret_file_contents() {
        let directory = tempfile::tempdir().expect("tempdir");
        let secret = directory.path().join(".env");
        let canary = b"OPENAI_API_KEY=sk-secret-canary";
        std::fs::write(&secret, canary).expect("write canary");

        let checksum = checksum_path(&secret).expect("metadata checksum");
        assert!(checksum.starts_with("metadata-fnv1a64:"));

        let mut content_digest = Fnv1a64::new();
        content_digest.update(canary);
        assert_ne!(
            checksum,
            format!("fnv1a64:{:016x}", content_digest.finish()),
            "manifest checksums must never be hashes of secret-bearing file contents"
        );
    }
}
