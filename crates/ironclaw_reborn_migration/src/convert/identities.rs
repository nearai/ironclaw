//! Identity converter (v1 `user_identities` + `channel_identities` → Reborn).
//!
//! Target is `RebornIdentityResolver::adopt_migrated_identity`, purpose-built to
//! carry a v1 identity over while preserving its `UserId` (idempotent;
//! first-writer-wins verified-email index). v1 OAuth/social identities
//! (`user_identities`) map to `SurfaceKind::Oauth`; channel actor mappings
//! (`channel_identities`, read raw since there is no `Database` accessor) map to
//! `SurfaceKind::ChannelActor`. `pairing_requests` has no Reborn store and is
//! recorded as a gap.

use ironclaw_host_api::UserId;
use ironclaw_reborn_identity::{
    ExternalSubjectId, ProviderKind, RebornIdentityResolver, ResolveExternalIdentity, SurfaceKind,
};

use crate::error::MigrationError;
use crate::legacy_snapshot::UserIdentityRecord;
use crate::options::MigrationOptions;
use crate::report::{Domain, LossReason, MigrationReport};
use crate::source::V1Source;
use crate::target::RebornTarget;

pub(crate) async fn run(
    src: &V1Source,
    tgt: &mut RebornTarget,
    options: &MigrationOptions,
    report: &mut MigrationReport,
) -> Result<(), MigrationError> {
    migrate_user_identities(src, tgt, options, report).await?;
    migrate_channel_identities(src, tgt, options, report).await?;

    report.record_loss(
        Domain::Identity,
        "pairing_requests",
        "*",
        LossReason::NoTargetConcept,
        "v1 pairing_requests has no Reborn durable store (pairing is handled live)".to_string(),
    );
    Ok(())
}

// ── user_identities (OAuth/social) via the Database trait ────────────────────

async fn migrate_user_identities(
    src: &V1Source,
    tgt: &mut RebornTarget,
    options: &MigrationOptions,
    report: &mut MigrationReport,
) -> Result<(), MigrationError> {
    // `list_identities_for_user` is per-user and there is no all-users accessor;
    // enumerate users from the users table (tolerant) unioned with data-derived
    // users so installs without a users table still resolve identities.
    let mut users: std::collections::BTreeSet<String> = src
        .distinct_user_ids_in("users", "id")
        .await?
        .into_iter()
        .collect();
    users.extend(src.distinct_users().await?);

    for user in users {
        let identities = match src.db.list_identities_for_user(&user).await {
            Ok(identities) => identities,
            Err(e) if crate::source::is_missing_table_error(&e.to_string()) => Vec::new(),
            Err(e) => {
                return Err(MigrationError::ReadSource {
                    domain: "user_identities".into(),
                    reason: e.to_string(),
                });
            }
        };
        if identities.is_empty() {
            continue;
        }
        // Consistent with `migrate_channel_identities` below: a malformed source
        // user id skips that user's identities but is recorded, not dropped
        // silently.
        let Some(host_user) =
            report.valid_user_id(Domain::Identity, format!("user:{user}"), "user_id", &user)
        else {
            continue;
        };
        let resolver = tgt.identity_store(host_user);

        for rec in identities {
            let identity = match build_oauth_identity(tgt, &rec, report) {
                Some(identity) => identity,
                None => continue,
            };
            adopt(&resolver, identity, &rec.user_id, options, report).await?;
        }
    }
    Ok(())
}

fn build_oauth_identity(
    tgt: &RebornTarget,
    rec: &UserIdentityRecord,
    report: &mut MigrationReport,
) -> Option<ResolveExternalIdentity> {
    let source_id = format!("identity:{}:{}", rec.provider, rec.provider_user_id);
    let provider_kind = match ProviderKind::new(rec.provider.clone()) {
        Ok(kind) => kind,
        Err(e) => {
            report.record_loss(
                Domain::Identity,
                &source_id,
                "provider",
                LossReason::Unparseable,
                format!("invalid provider kind: {e}"),
            );
            return None;
        }
    };
    let subject = match ExternalSubjectId::new(rec.provider_user_id.clone()) {
        Ok(subject) => subject,
        Err(e) => {
            report.record_loss(
                Domain::Identity,
                &source_id,
                "provider_user_id",
                LossReason::Unparseable,
                format!("invalid external subject id: {e}"),
            );
            return None;
        }
    };
    Some(ResolveExternalIdentity {
        tenant_id: tgt.tenant_id.clone(),
        surface_kind: SurfaceKind::Oauth,
        provider_kind,
        provider_instance_id: None,
        external_subject_id: subject,
        email: rec.email.clone(),
        email_verified: rec.email_verified,
        display_name: rec.display_name.clone(),
    })
}

// ── channel_identities (channel actors) via raw SQL ──────────────────────────

async fn migrate_channel_identities(
    src: &V1Source,
    tgt: &mut RebornTarget,
    options: &MigrationOptions,
    report: &mut MigrationReport,
) -> Result<(), MigrationError> {
    let rows = read_channel_identities(src, report).await?;
    for (owner_id, channel, external_id) in rows {
        let source_id = format!("channel_identity:{channel}:{external_id}");
        let Some(host_user) =
            report.valid_user_id(Domain::Identity, &source_id, "owner_id", &owner_id)
        else {
            continue;
        };
        let (Ok(provider_kind), Ok(subject)) = (
            ProviderKind::new(channel.clone()),
            ExternalSubjectId::new(external_id.clone()),
        ) else {
            report.record_loss(
                Domain::Identity,
                &source_id,
                "channel/external_id",
                LossReason::Unparseable,
                "invalid channel or external subject id".to_string(),
            );
            continue;
        };
        let resolver = tgt.identity_store(host_user);
        let identity = ResolveExternalIdentity {
            tenant_id: tgt.tenant_id.clone(),
            surface_kind: SurfaceKind::ChannelActor,
            provider_kind,
            provider_instance_id: None,
            external_subject_id: subject,
            email: None,
            email_verified: false,
            display_name: None,
        };
        adopt(&resolver, identity, &owner_id, options, report).await?;
    }
    Ok(())
}

/// Raw read of `channel_identities` (no `Database` accessor exists).
///
/// Only an **absent table** is tolerated (returns empty) — v1 installs without
/// the table legitimately have no channel identities. Connect / query failures
/// are real infrastructure errors and propagate. A row that fails to decode is
/// recorded as a per-row loss rather than silently skipped.
async fn read_channel_identities(
    src: &V1Source,
    report: &mut MigrationReport,
) -> Result<Vec<(String, String, String)>, MigrationError> {
    let sql = "SELECT owner_id, channel, external_id FROM channel_identities";
    let read_err = |e: &dyn std::fmt::Display| MigrationError::ReadSource {
        domain: "channel_identities".to_string(),
        reason: e.to_string(),
    };
    let record_bad_row = |report: &mut MigrationReport, e: &dyn std::fmt::Display| {
        report.record_loss(
            Domain::Identity,
            "channel_identities",
            "row",
            LossReason::Unparseable,
            format!("channel_identities row could not be decoded (skipped): {e}"),
        );
    };
    if let Some(db) = src.handles.libsql_db.as_ref() {
        let conn = db.connect().map_err(|e| read_err(&e))?;
        let mut rows = match conn.query(sql, ()).await {
            Ok(rows) => rows,
            Err(e) if crate::source::is_missing_table_error(&e.to_string()) => {
                return Ok(Vec::new());
            }
            Err(e) => return Err(read_err(&e)),
        };
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(|e| read_err(&e))? {
            match (
                row.get::<String>(0),
                row.get::<String>(1),
                row.get::<String>(2),
            ) {
                (Ok(owner), Ok(channel), Ok(external)) => out.push((owner, channel, external)),
                (Err(e), ..) | (_, Err(e), _) | (.., Err(e)) => record_bad_row(report, &e),
            }
        }
        return Ok(out);
    }
    if let Some(pool) = src.handles.pg_pool.as_ref() {
        let client = pool.get().await.map_err(|e| read_err(&e))?;
        let rows = match client.query(sql, &[]).await {
            Ok(rows) => rows,
            Err(e) if crate::source::is_missing_postgres_table_error(&e) => return Ok(Vec::new()),
            Err(e) => return Err(read_err(&e)),
        };
        let mut out = Vec::new();
        for row in &rows {
            match (
                row.try_get::<_, String>(0),
                row.try_get::<_, String>(1),
                row.try_get::<_, String>(2),
            ) {
                (Ok(owner), Ok(channel), Ok(external)) => out.push((owner, channel, external)),
                (Err(e), ..) | (_, Err(e), _) | (.., Err(e)) => record_bad_row(report, &e),
            }
        }
        return Ok(out);
    }
    Ok(Vec::new())
}

async fn adopt(
    resolver: &std::sync::Arc<dyn RebornIdentityResolver>,
    identity: ResolveExternalIdentity,
    migrated_user_id: &str,
    options: &MigrationOptions,
    report: &mut MigrationReport,
) -> Result<(), MigrationError> {
    let migrated_user = UserId::new(migrated_user_id).map_err(|e| MigrationError::WriteTarget {
        domain: format!("identity migrated user_id {migrated_user_id}"),
        reason: e.to_string(),
    })?;
    if !options.dry_run {
        resolver
            .adopt_migrated_identity(identity, &migrated_user)
            .await
            .map_err(|e| MigrationError::WriteTarget {
                domain: format!("identity for {migrated_user_id}"),
                reason: e.to_string(),
            })?;
    }
    report.stats.identities += 1;
    Ok(())
}
