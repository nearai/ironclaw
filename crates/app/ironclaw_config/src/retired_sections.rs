//! Retired `config.toml` sections — the compatibility window.
//!
//! # Why this module exists, and why it is here rather than in a package
//!
//! `config.toml` once carried per-vendor sections that configured a channel
//! directly (`[slack]`, `[telegram]`). Under the unified extension model an
//! extension is installed and authorized through its own lifecycle, so those
//! sections have no runtime reader left. Deleting them from the schema
//! outright would make every existing operator file fail to parse, because
//! [`crate::RebornConfigFile`] is `deny_unknown_fields` — a typo-catching
//! property worth keeping.
//!
//! So the sections are *retired*, not deleted: an operator's existing file
//! still parses, and the operator is told precisely what to do instead. This
//! is the deprecation window PROPOSAL §12.2 names as a compatibility
//! constraint.
//!
//! **Ownership note.** A retired section is a fact about *this crate's own
//! schema history*, not about the extension that once used it: the check runs
//! at config-parse time, before any extension exists to own it, and this crate
//! may not depend on any IronClaw workspace crate. Live admin configuration
//! for an extension is package-owned (the manifest `[admin_configuration]`
//! model); the gravestone for a schema key this crate used to define stays
//! here. Adding a row below is the whole cost of retiring a future section.
//!
//! # The two tiers
//!
//! A retired section is not automatically an error, because failing a boot
//! that used to work is a worse outcome than ignoring a stale key:
//!
//! - **Rejected keys** — the operator is following retired *setup*
//!   instructions. The value would be silently ignored, so the boot fails
//!   closed with a migration pointer instead.
//! - **Anything else in the section** — inert. The boot continues and emits a
//!   deprecation notice, so an operator who set an advertised-but-unread flag
//!   learns that it does nothing rather than believing it took effect.

use std::collections::BTreeMap;

use thiserror::Error;

/// One retired top-level `config.toml` section.
pub(crate) struct RetiredSectionPolicy {
    /// The table name exactly as an operator would have written it.
    pub section: &'static str,
    /// Keys whose presence fails the boot closed. These are the *setup*
    /// fields: a value here means the operator followed instructions that no
    /// longer connect to anything, and silently ignoring it would leave them
    /// debugging a channel that never activates.
    pub rejected_keys: &'static [&'static str],
    /// What to do instead. Appended to both the hard error and the
    /// deprecation notice, so the two never drift apart.
    pub migration: &'static str,
}

/// Every retired section. Adding a row is how a future section is retired.
pub(crate) const RETIRED_SECTIONS: &[RetiredSectionPolicy] = &[
    RetiredSectionPolicy {
        section: "slack",
        rejected_keys: &[
            "installation_id",
            "team_id",
            "api_app_id",
            "slack_user_id",
            "user_id",
            "shared_subject_user_id",
            "channel_routes",
            "signing_secret_env",
            "bot_token_env",
        ],
        migration: "Slack is configured by installing the Slack extension and completing workspace \
             OAuth in the WebUI (/extensions).",
    },
    RetiredSectionPolicy {
        section: "telegram",
        // `enabled` was the only key this section ever accepted, and nothing
        // has read it since the unified extension runtime landed. There is no
        // setup field to fail closed on, so the whole section is inert.
        rejected_keys: &[],
        migration: "Telegram is configured by installing the Telegram extension and completing bot \
             setup in the WebUI (/extensions).",
    },
];

fn policy_for(section: &str) -> Option<&'static RetiredSectionPolicy> {
    RETIRED_SECTIONS
        .iter()
        .find(|policy| policy.section == section)
}

/// Migration guidance for a dotted `config set <key>` argument whose section
/// is retired, or `None` for a key this crate has nothing to say about.
///
/// Driven by the same table as the boot-time check so a caller cannot answer
/// `config set slack.enabled` and `serve` with two different stories. Without
/// it, a retired key falls through to the generic "unknown config key" list,
/// which tells an operator following an old runbook that they typed something
/// wrong rather than that the setting is gone.
pub fn retired_config_key_guidance(key: &str) -> Option<String> {
    let (section, _) = key.split_once('.')?;
    let policy = policy_for(section)?;
    Some(format!(
        "`{key}` is a retired configuration key and is no longer read. {migration}",
        key = key,
        migration = policy.migration,
    ))
}

/// The retired sections an operator's file actually carried.
///
/// Captured verbatim rather than parsed into a typed shape: the point is to
/// recognize the section and explain it, not to model fields nothing reads.
#[derive(Debug, Clone, Default)]
pub struct RetiredSections {
    entries: BTreeMap<String, toml::Table>,
}

impl RetiredSections {
    /// Remove every retired section from a raw parsed document, returning
    /// what was found. The caller deserializes what is left, so the typed
    /// schema never has to name a retired section.
    pub(crate) fn split_from(raw: &mut toml::Table) -> Self {
        let mut entries = BTreeMap::new();
        for policy in RETIRED_SECTIONS {
            let Some(value) = raw.remove(policy.section) else {
                continue;
            };
            // A non-table `slack = 1` is not a retired *section*; put it back
            // so the typed parse reports it as the unknown field it is.
            match value {
                toml::Value::Table(table) => {
                    entries.insert(policy.section.to_string(), table);
                }
                other => {
                    raw.insert(policy.section.to_string(), other);
                }
            }
        }
        Self { entries }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Section names present, in a stable order.
    pub fn section_names(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// Fail closed when a retired *setup* key is present.
    ///
    /// Reports the first offending key in declaration order so the message is
    /// deterministic; the operator is told to remove the whole section anyway.
    pub fn migration_error(
        &self,
        config_path: &std::path::Path,
    ) -> Result<(), RetiredSectionError> {
        for (section, table) in &self.entries {
            let Some(policy) = policy_for(section) else {
                continue;
            };
            for key in policy.rejected_keys {
                if !table.contains_key(*key) {
                    continue;
                }
                return Err(RetiredSectionError {
                    section: section.clone(),
                    field: (*key).to_string(),
                    path: config_path.display().to_string(),
                    migration: policy.migration.to_string(),
                });
            }
        }
        Ok(())
    }

    /// One notice per inert retired section still present in the file.
    ///
    /// Emitted rather than swallowed because the failure mode this replaces is
    /// an operator setting a documented flag and believing it took effect.
    pub fn deprecation_notices(&self, config_path: &std::path::Path) -> Vec<String> {
        self.entries
            .keys()
            .filter_map(|section| policy_for(section).map(|policy| (section, policy)))
            .map(|(section, policy)| {
                format!(
                    "`[{section}]` in {path} is a retired configuration section and is ignored. \
                     {migration} Remove the `[{section}]` section to silence this notice.",
                    section = section,
                    path = config_path.display(),
                    migration = policy.migration,
                )
            })
            .collect()
    }

    /// Every string value in every retired section, keyed by dotted path.
    ///
    /// Retired sections skip the typed schema, so they would also skip the
    /// inline-secret guard that runs over it. Walking them here keeps a
    /// pasted secret rejected no matter which section it landed in — a
    /// strictly wider net than the per-field checks this replaced, which knew
    /// only the nine Slack keys.
    pub(crate) fn string_values(&self) -> Vec<(String, &str)> {
        let mut found = Vec::new();
        for (section, table) in &self.entries {
            collect_table_strings(table, section, &mut found);
        }
        found
    }
}

fn collect_table_strings<'a>(
    table: &'a toml::Table,
    prefix: &str,
    found: &mut Vec<(String, &'a str)>,
) {
    for (key, value) in table {
        collect_value_strings(value, &format!("{prefix}.{key}"), found);
    }
}

fn collect_value_strings<'a>(
    value: &'a toml::Value,
    path: &str,
    found: &mut Vec<(String, &'a str)>,
) {
    match value {
        toml::Value::String(text) => found.push((path.to_string(), text.as_str())),
        toml::Value::Table(table) => collect_table_strings(table, path, found),
        toml::Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                collect_value_strings(item, &format!("{path}[{index}]"), found);
            }
        }
        _ => {}
    }
}

/// A retired setup key was present, so the boot fails closed.
#[derive(Debug, Error)]
#[error(
    "`[{section}].{field}` in {path} is a retired configuration surface: {migration} Remove the \
     `[{section}]` section to continue."
)]
pub struct RetiredSectionError {
    pub section: String,
    pub field: String,
    pub path: String,
    pub migration: String,
}
