//! v1 canonical users -> Reborn canonical user directory.
//!
//! User ids and lifecycle timestamps are preserved. Reborn has no
//! `deactivated` state, so deactivated (and unknown) source statuses map to
//! `Suspended`, never `Active`. Unknown roles map to `Member`, never an admin
//! role. v1 API tokens contain only one-way hashes and are therefore reported
//! as requiring re-authentication; this converter never reads or writes token
//! hashes.

use std::collections::{BTreeMap, BTreeSet};

use ironclaw::db::{ApiTokenRecord, UserRecord};
use ironclaw_host_api::UserId;
use ironclaw_reborn_identity::{RebornUser, RebornUserRole, RebornUserStatus};

use crate::error::MigrationError;
use crate::options::MigrationOptions;
use crate::report::{Domain, LossReason, MigrationReport};
use crate::source::V1Source;
use crate::target::RebornTarget;

pub(crate) async fn run(
    src: &V1Source,
    tgt: &mut RebornTarget,
    _options: &MigrationOptions,
    report: &mut MigrationReport,
) -> Result<(), MigrationError> {
    let (users, canonical_users_table_present) = match src.db.list_users(None).await {
        Ok(users) => (users, true),
        Err(error) if crate::source::is_missing_table_error(&error.to_string()) => {
            (Vec::new(), false)
        }
        Err(error) => {
            return Err(MigrationError::ReadSource {
                domain: "users".to_string(),
                reason: error.to_string(),
            });
        }
    };

    let mut canonical_ids = BTreeSet::new();
    for source in users {
        canonical_ids.insert(source.id.clone());
        report_api_tokens(src, &source, report).await?;

        let Some(user) = build_user(tgt, source, report) else {
            continue;
        };
        import_user(tgt, user, report).await?;
    }

    // Import deterministic minimal users for ids that already own durable data
    // so migrated records never point at a missing user. Only schemas without
    // a canonical users table can safely treat those owners as active.
    for raw_id in src.distinct_users().await? {
        if canonical_ids.contains(&raw_id) {
            continue;
        }
        let source_id = format!("data_owner:{raw_id}");
        let Some(user_id) = report.valid_user_id(Domain::User, &source_id, "user_id", &raw_id)
        else {
            continue;
        };
        import_user(
            tgt,
            RebornUser {
                user_id,
                email: None,
                display_name: Some(raw_id),
                status: if canonical_users_table_present {
                    report.record_loss(
                        Domain::User,
                        &source_id,
                        "status",
                        LossReason::Degraded,
                        "durable-data owner is absent from the canonical users table; synthesized fail-closed as suspended",
                    );
                    RebornUserStatus::Suspended
                } else {
                    RebornUserStatus::Active
                },
                role: RebornUserRole::Member,
                created_at: "1970-01-01T00:00:00+00:00".to_string(),
                updated_at: "1970-01-01T00:00:00+00:00".to_string(),
                created_by: None,
                last_login_at: None,
                tenant_id: Some(tgt.tenant_id.clone()),
                metadata: BTreeMap::from([
                    (
                        "migration.source".to_string(),
                        "ironclaw-v1-data-owner".to_string(),
                    ),
                    ("migration.synthesized".to_string(), "true".to_string()),
                ]),
            },
            report,
        )
        .await?;
    }

    Ok(())
}

async fn import_user(
    tgt: &RebornTarget,
    user: RebornUser,
    report: &mut MigrationReport,
) -> Result<(), MigrationError> {
    let directory = tgt.user_directory(user.user_id.clone());
    directory
        .import_migrated_user(user.clone())
        .await
        .map_err(|error| MigrationError::WriteTarget {
            domain: format!("user {}", user.user_id),
            reason: error.to_string(),
        })?;
    report.stats.users += 1;
    Ok(())
}

fn build_user(
    tgt: &RebornTarget,
    source: UserRecord,
    report: &mut MigrationReport,
) -> Option<RebornUser> {
    let source_id = format!("user:{}", source.id);
    let user_id = report.valid_user_id(Domain::User, &source_id, "id", &source.id)?;
    let status = map_status(&source.status, &source_id, report);
    let role = map_role(&source.role, &source_id, report);
    let created_by = source.created_by.as_deref().and_then(|raw| {
        UserId::new(raw).map_or_else(
            |error| {
                report.record_loss(
                    Domain::User,
                    &source_id,
                    "created_by",
                    LossReason::Unparseable,
                    format!(
                        "source created_by is not a valid Reborn UserId and was omitted: {error}"
                    ),
                );
                None
            },
            Some,
        )
    });
    let metadata = map_metadata(source.metadata, &source_id, report);

    Some(RebornUser {
        user_id,
        email: source.email,
        display_name: Some(source.display_name),
        status,
        role,
        created_at: source.created_at.to_rfc3339(),
        updated_at: source.updated_at.to_rfc3339(),
        created_by,
        last_login_at: source.last_login_at.map(|at| at.to_rfc3339()),
        tenant_id: Some(tgt.tenant_id.clone()),
        metadata,
    })
}

fn map_status(source: &str, source_id: &str, report: &mut MigrationReport) -> RebornUserStatus {
    match source.trim().to_ascii_lowercase().as_str() {
        "active" => RebornUserStatus::Active,
        "suspended" => RebornUserStatus::Suspended,
        "deactivated" => {
            report.record_loss(
                Domain::User,
                source_id,
                "status",
                LossReason::Degraded,
                "v1 deactivated has no Reborn equivalent; mapped fail-closed to suspended",
            );
            RebornUserStatus::Suspended
        }
        _ => {
            report.record_loss(
                Domain::User,
                source_id,
                "status",
                LossReason::Unparseable,
                format!("unsupported v1 user status {source:?}; mapped fail-closed to suspended"),
            );
            RebornUserStatus::Suspended
        }
    }
}

fn map_role(source: &str, source_id: &str, report: &mut MigrationReport) -> RebornUserRole {
    match source.trim().to_ascii_lowercase().as_str() {
        "admin" => RebornUserRole::Admin,
        "member" => RebornUserRole::Member,
        _ => {
            report.record_loss(
                Domain::User,
                source_id,
                "role",
                LossReason::Unparseable,
                format!("unsupported v1 user role {source:?}; mapped fail-closed to member"),
            );
            RebornUserRole::Member
        }
    }
}

fn map_metadata(
    source: serde_json::Value,
    source_id: &str,
    report: &mut MigrationReport,
) -> BTreeMap<String, String> {
    let serde_json::Value::Object(values) = source else {
        report.record_loss(
            Domain::User,
            source_id,
            "metadata",
            LossReason::Unparseable,
            "v1 user metadata was not a JSON object and was omitted",
        );
        return BTreeMap::new();
    };

    values
        .into_iter()
        .map(|(key, value)| match value {
            serde_json::Value::String(value) => (key, value),
            value => {
                report.record_loss(
                    Domain::User,
                    source_id,
                    format!("metadata.{key}"),
                    LossReason::Degraded,
                    "non-string metadata value was preserved as compact JSON text",
                );
                (key, value.to_string())
            }
        })
        .collect()
}

async fn report_api_tokens(
    src: &V1Source,
    user: &UserRecord,
    report: &mut MigrationReport,
) -> Result<(), MigrationError> {
    let tokens = match src.db.list_api_tokens(&user.id).await {
        Ok(tokens) => tokens,
        Err(error) if crate::source::is_missing_table_error(&error.to_string()) => Vec::new(),
        Err(error) => {
            return Err(MigrationError::ReadSource {
                domain: "api_tokens".to_string(),
                reason: error.to_string(),
            });
        }
    };
    for token in tokens {
        record_token_reauth(token, report);
    }
    Ok(())
}

fn record_token_reauth(token: ApiTokenRecord, report: &mut MigrationReport) {
    report.record_loss(
        Domain::ApiToken,
        format!("api_token:{}", token.id),
        "*",
        LossReason::NoTargetConcept,
        "v1 API tokens store only one-way hashes and cannot be reused; issue a new Reborn credential after cutover",
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_lifecycle_values_map_fail_closed() {
        let mut report = MigrationReport::new(false);
        assert_eq!(
            map_status("deactivated", "user:a", &mut report),
            RebornUserStatus::Suspended
        );
        assert_eq!(
            map_status("future-state", "user:a", &mut report),
            RebornUserStatus::Suspended
        );
        assert_eq!(
            map_role("owner", "user:a", &mut report),
            RebornUserRole::Member
        );
        assert_eq!(report.losses_in(Domain::User), 3);
    }

    #[test]
    fn metadata_preserves_strings_and_serializes_other_json_values() {
        let mut report = MigrationReport::new(false);
        let metadata = map_metadata(
            serde_json::json!({"team": "infra", "quota": 3, "nested": {"a": true}}),
            "user:a",
            &mut report,
        );
        assert_eq!(metadata.get("team").map(String::as_str), Some("infra"));
        assert_eq!(metadata.get("quota").map(String::as_str), Some("3"));
        assert_eq!(
            metadata.get("nested").map(String::as_str),
            Some("{\"a\":true}")
        );
        assert_eq!(report.losses_in(Domain::User), 2);
    }
}
