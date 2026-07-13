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
        disposition: Disposition::Unsupported,
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
        disposition: Disposition::Unsupported,
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
        disposition: Disposition::Unsupported,
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
        disposition: Disposition::Unsupported,
    },
    DispositionRule {
        name: "settings.json",
        domain: Domain::Setting,
        disposition: Disposition::Unsupported,
    },
    DispositionRule {
        name: "config.toml",
        domain: Domain::Setting,
        disposition: Disposition::Unsupported,
    },
    DispositionRule {
        name: "providers.json",
        domain: Domain::Provider,
        disposition: Disposition::Unsupported,
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
        disposition: Disposition::Unsupported,
    },
    DispositionRule {
        name: "skills",
        domain: Domain::Skill,
        disposition: Disposition::Unsupported,
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
        disposition: Disposition::Unsupported,
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
        .is_some_and(valid_audit_partition_suffix)
    {
        return Some(DispositionRule {
            name: "secret_usage_log_partition",
            domain: Domain::SecurityAudit,
            disposition: Disposition::ArchiveOnly,
        });
    }
    None
}

fn valid_audit_partition_suffix(suffix: &str) -> bool {
    let bytes = suffix.as_bytes();
    let [year_a, year_b, year_c, year_d, b'm', month_a, month_b] = bytes else {
        return false;
    };
    if ![year_a, year_b, year_c, year_d, month_a, month_b]
        .into_iter()
        .all(u8::is_ascii_digit)
    {
        return false;
    }
    let month = (*month_a - b'0') * 10 + (*month_b - b'0');
    (1..=12).contains(&month)
}

pub(crate) fn build_home_inventory(
    home: Option<&Path>,
    source_db: Option<&Path>,
    target_db: Option<&Path>,
) -> Vec<InventoryEntry> {
    let mut out = Vec::new();
    let Some(home) = home else { return out };
    let mut excluded_paths = BTreeSet::new();
    for path in [source_db, target_db].into_iter().flatten() {
        for suffix in ["", "-wal", "-shm"] {
            let mut candidate = path.as_os_str().to_os_string();
            candidate.push(suffix);
            excluded_paths.insert(normalized_path(Path::new(&candidate)));
        }
    }
    if let Some(target_db) = target_db
        && let Some(parent) = target_db.parent()
    {
        excluded_paths.insert(normalized_path(
            &parent.join(".reborn-local-dev-secrets-master-key"),
        ));
    }

    for rule in FILE_RULES {
        out.push(home_entry(
            home.join(rule.name),
            InventorySourceKind::HomeFile,
            *rule,
            &excluded_paths,
        ));
    }
    for rule in DIRECTORY_RULES {
        out.push(home_entry(
            home.join(rule.name),
            InventorySourceKind::HomeDirectory,
            *rule,
            &excluded_paths,
        ));
    }

    let known: BTreeSet<&str> = FILE_RULES
        .iter()
        .chain(DIRECTORY_RULES)
        .map(|rule| rule.name)
        .collect();
    match std::fs::read_dir(home) {
        Ok(entries) => {
            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) => {
                        out.push(home_inventory_error("home_directory_entry", &error));
                        continue;
                    }
                };
                let name = entry.file_name().to_string_lossy().into_owned();
                if known.contains(name.as_str())
                    || excluded_paths.contains(&normalized_path(&entry.path()))
                {
                    continue;
                }
                let kind = match entry.file_type() {
                    Ok(kind) if kind.is_dir() => InventorySourceKind::HomeDirectory,
                    Ok(_) => InventorySourceKind::HomeFile,
                    Err(error) => {
                        out.push(home_inventory_error("home_entry_type", &error));
                        continue;
                    }
                };
                let normalized_entry = normalized_path(&entry.path());
                if matches!(kind, InventorySourceKind::HomeDirectory)
                    && excluded_paths
                        .iter()
                        .any(|excluded| excluded.starts_with(&normalized_entry))
                    && matches!(
                        path_has_nonexcluded_content(&entry.path(), &excluded_paths),
                        Ok(false)
                    )
                {
                    continue;
                }
                let (count, count_error) =
                    match count_path_entries_excluding(&entry.path(), &excluded_paths) {
                        Ok(count) => (count, None),
                        Err(error) => (0, Some(error)),
                    };
                let (checksum, checksum_error) =
                    match checksum_path_excluding(&entry.path(), &excluded_paths) {
                        Ok(checksum) => (checksum, None),
                        Err(error) => (
                            sha256_hex(format!("unreadable-home:{name}:{error}").as_bytes()),
                            Some(error),
                        ),
                    };
                let blocker = count_error.or(checksum_error).map(|error| {
                    format!("v1 home artifact could not be inventoried completely: {error}")
                });
                out.push(InventoryEntry {
                    source_kind: kind,
                    source_name: name.clone(),
                    domain: Domain::Unknown,
                    disposition: Disposition::UnsupportedUnknown,
                    count,
                    checksum,
                    blocker: blocker.or_else(|| {
                        Some(
                            "unknown v1 home artifact requires an explicit migration disposition"
                                .to_string(),
                        )
                    }),
                    warning: None,
                });
            }
        }
        Err(error) => out.push(home_inventory_error("home_directory_enumeration", &error)),
    }
    out
}

fn home_entry(
    path: PathBuf,
    source_kind: InventorySourceKind,
    rule: DispositionRule,
    excluded_paths: &BTreeSet<PathBuf>,
) -> InventoryEntry {
    if excluded_paths.contains(&normalized_path(&path)) {
        return InventoryEntry {
            source_kind,
            source_name: rule.name.to_string(),
            domain: rule.domain,
            disposition: rule.disposition,
            count: 0,
            checksum: sha256_hex(b"excluded-database-path"),
            blocker: None,
            warning: None,
        };
    }
    match path.try_exists() {
        Ok(false) => {
            return InventoryEntry {
                source_kind,
                source_name: rule.name.to_string(),
                domain: rule.domain,
                disposition: rule.disposition,
                count: 0,
                checksum: sha256_hex(b"missing"),
                blocker: None,
                warning: None,
            };
        }
        Ok(true) => {}
        Err(error) => {
            return InventoryEntry {
                source_kind,
                source_name: rule.name.to_string(),
                domain: rule.domain,
                disposition: rule.disposition,
                count: 0,
                checksum: sha256_hex(format!("unreadable:{}:{error}", rule.name).as_bytes()),
                blocker: Some(format!(
                    "known v1 home artifact could not be inventoried completely: {error}"
                )),
                warning: None,
            };
        }
    }
    if matches!(
        path_has_nonexcluded_content(&path, excluded_paths),
        Ok(false)
    ) {
        return InventoryEntry {
            source_kind,
            source_name: rule.name.to_string(),
            domain: rule.domain,
            disposition: rule.disposition,
            count: 0,
            checksum: sha256_hex(b"missing"),
            blocker: None,
            warning: None,
        };
    }
    let (count, count_error) = match count_path_entries_excluding(&path, excluded_paths) {
        Ok(count) => (count, None),
        Err(error) => (0, Some(error)),
    };
    let (checksum, checksum_error) = match checksum_path_excluding(&path, excluded_paths) {
        Ok(checksum) => (checksum, None),
        Err(error) => (
            sha256_hex(format!("unreadable:{}:{error}", rule.name).as_bytes()),
            Some(error),
        ),
    };
    InventoryEntry {
        source_kind,
        source_name: rule.name.to_string(),
        domain: rule.domain,
        disposition: rule.disposition,
        count,
        checksum,
        blocker: count_error.or(checksum_error).map(|error| {
            format!("known v1 home artifact could not be inventoried completely: {error}")
        }),
        warning: None,
    }
}

fn home_inventory_error(source_name: &str, error: &std::io::Error) -> InventoryEntry {
    InventoryEntry {
        source_kind: InventorySourceKind::HomeDirectory,
        source_name: source_name.to_string(),
        domain: Domain::Unknown,
        disposition: Disposition::UnsupportedUnknown,
        count: 0,
        checksum: sha256_hex(format!("unreadable-home:{source_name}:{error}").as_bytes()),
        blocker: Some(format!(
            "v1 home directory could not be inventoried completely: {error}"
        )),
        warning: None,
    }
}

#[cfg(test)]
fn checksum_path(path: &Path) -> std::io::Result<String> {
    checksum_path_excluding(path, &BTreeSet::new())
}

fn checksum_path_excluding(
    path: &Path,
    excluded_paths: &BTreeSet<PathBuf>,
) -> std::io::Result<String> {
    let metadata = std::fs::symlink_metadata(path)?;
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
            if excluded_paths.contains(&normalized_path(&entry.path())) {
                continue;
            }
            state.update(entry.file_name().as_encoded_bytes());
            state.update(b"\0");
            state.update(checksum_path_excluding(&entry.path(), excluded_paths)?.as_bytes());
            state.update(b"\0");
        }
    } else {
        state.update(b"other\0");
    }
    let normalized = normalized_path(path);
    if !metadata.is_dir()
        || !excluded_paths
            .iter()
            .any(|excluded| excluded.starts_with(&normalized))
    {
        update_metadata_shape(&mut state, &metadata);
    }
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

#[cfg(test)]
fn count_path_entries(path: &Path) -> std::io::Result<u64> {
    count_path_entries_excluding(path, &BTreeSet::new())
}

fn count_path_entries_excluding(
    path: &Path,
    excluded_paths: &BTreeSet<PathBuf>,
) -> std::io::Result<u64> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Ok(1);
    }
    let mut count = 1_u64;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if excluded_paths.contains(&normalized_path(&entry.path())) {
            continue;
        }
        count = count.saturating_add(count_path_entries_excluding(&entry.path(), excluded_paths)?);
    }
    Ok(count)
}

fn normalized_path(path: &Path) -> PathBuf {
    crate::canonicalish(path)
}

fn path_has_nonexcluded_content(
    path: &Path,
    excluded_paths: &BTreeSet<PathBuf>,
) -> std::io::Result<bool> {
    if excluded_paths.contains(&normalized_path(path)) {
        return Ok(false);
    }
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Ok(true);
    }
    for entry in std::fs::read_dir(path)? {
        if path_has_nonexcluded_content(&entry?.path(), excluded_paths)? {
            return Ok(true);
        }
    }
    Ok(false)
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
    fn audit_partition_names_require_exact_year_and_valid_month() {
        for valid in ["2024m01", "9999m12"] {
            assert!(valid_audit_partition_suffix(valid), "{valid}");
        }
        for invalid in [
            "ABCDm12", "2024m00", "2024m13", "2024x01", "2024m1", "20240m1",
        ] {
            assert!(!valid_audit_partition_suffix(invalid), "{invalid}");
        }
    }

    #[test]
    fn heartbeat_state_is_reported_as_unsupported() {
        let inventory = build_table_inventory(vec![RawTableInventory {
            name: "heartbeat_state".to_string(),
            count: 1,
            checksum: "checksum".to_string(),
        }]);
        let entry = inventory
            .iter()
            .find(|entry| entry.source_name == "heartbeat_state")
            .expect("heartbeat inventory entry");
        assert_eq!(entry.disposition, Disposition::Unsupported);
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

    #[test]
    fn unconverted_known_sources_are_never_labeled_semantically_converted() {
        for name in [
            "settings",
            "root_filesystem_entries",
            ".env",
            "settings.json",
            "config.toml",
            "providers.json",
            "profiles",
            "skills",
            "projects",
        ] {
            let rule = TABLE_RULES
                .iter()
                .chain(FILE_RULES)
                .chain(DIRECTORY_RULES)
                .find(|rule| rule.name == name)
                .expect("known source rule");
            assert_eq!(rule.disposition, Disposition::Unsupported, "{name}");
        }
    }

    #[test]
    fn unreadable_home_enumeration_is_a_blocker() {
        let directory = tempfile::tempdir().expect("tempdir");
        let not_a_directory = directory.path().join("home-file");
        std::fs::write(&not_a_directory, b"not a directory").expect("write file");

        let inventory = build_home_inventory(Some(&not_a_directory), None, None);
        let failure = inventory
            .iter()
            .find(|entry| entry.source_name == "home_directory_enumeration")
            .expect("enumeration blocker");
        assert_eq!(failure.disposition, Disposition::UnsupportedUnknown);
        assert!(failure.blocker.is_some());
    }

    #[test]
    fn vanished_inventory_path_is_an_error() {
        let directory = tempfile::tempdir().expect("tempdir");
        let vanished = directory.path().join("vanished");
        let error = count_path_entries(&vanished).expect_err("missing path must fail");
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn database_exclusion_is_exact_and_preserves_nested_siblings() {
        let directory = tempfile::tempdir().expect("tempdir");
        let home = directory.path().join("home");
        let nested = home.join("custom");
        std::fs::create_dir_all(&nested).expect("create nested directory");
        let source = nested.join("source.db");
        std::fs::write(&source, b"database").expect("write source");
        std::fs::write(nested.join("notes.txt"), b"notes").expect("write sibling");
        std::fs::write(home.join("source.db"), b"unrelated").expect("write same-name artifact");

        let inventory = build_home_inventory(Some(&home), Some(&source), None);
        let nested_entry = inventory
            .iter()
            .find(|entry| entry.source_name == "custom")
            .expect("nested directory remains inventoried");
        assert_eq!(nested_entry.count, 2, "directory plus non-database sibling");
        assert!(
            inventory
                .iter()
                .any(|entry| entry.source_name == "source.db"),
            "same basename at a different path must remain visible"
        );
    }

    #[test]
    fn target_only_directories_do_not_change_source_home_inventory() {
        let directory = tempfile::tempdir().expect("tempdir");
        let home = directory.path().join("home");
        let target = home.join("reborn").join("reborn.db");
        std::fs::create_dir_all(&home).expect("create home");
        let before = build_home_inventory(Some(&home), None, Some(&target));

        std::fs::create_dir_all(target.parent().expect("target parent"))
            .expect("create target parent");
        std::fs::write(&target, b"target").expect("write target");
        std::fs::write(
            target
                .parent()
                .expect("target parent")
                .join(".reborn-local-dev-secrets-master-key"),
            b"key",
        )
        .expect("write target key");
        let after = build_home_inventory(Some(&home), None, Some(&target));

        assert_eq!(after, before);
    }

    #[test]
    fn target_tree_under_known_directory_does_not_change_inventory() {
        let directory = tempfile::tempdir().expect("tempdir");
        let home = directory.path().join("home");
        let target = home.join("projects").join("reborn").join("reborn.db");
        std::fs::create_dir_all(&home).expect("create home");
        let before = build_home_inventory(Some(&home), None, Some(&target));

        std::fs::create_dir_all(target.parent().expect("target parent"))
            .expect("create target parent");
        std::fs::write(&target, b"target").expect("write target");
        let after = build_home_inventory(Some(&home), None, Some(&target));

        assert_eq!(after, before);
    }

    #[test]
    fn target_tree_exclusion_preserves_known_directory_siblings() {
        let directory = tempfile::tempdir().expect("tempdir");
        let home = directory.path().join("home");
        let projects = home.join("projects");
        let target = projects.join("reborn.db");
        std::fs::create_dir_all(&projects).expect("create projects");
        std::fs::write(projects.join("legacy.json"), b"legacy").expect("write legacy sibling");
        let before = build_home_inventory(Some(&home), None, Some(&target));

        std::fs::create_dir_all(target.parent().expect("target parent"))
            .expect("create target parent");
        std::fs::write(&target, b"target").expect("write target");
        let after = build_home_inventory(Some(&home), None, Some(&target));

        assert_eq!(after, before);
    }
}
