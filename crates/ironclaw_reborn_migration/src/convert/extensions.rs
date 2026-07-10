//! v1 extension inventory and safe disposition.
//!
//! A v1 WASM row is not a runnable Reborn extension package: the source row
//! does not carry a Reborn manifest, package assets, schemas, prompt docs, or a
//! catalog identity. Production restore resolves *every* installation through
//! the available-extension catalog, including disabled installations. Writing
//! a synthesized placeholder would therefore make a later cold boot fail.
//!
//! Until a converter can resolve a v1 artifact to a real bundled package, this
//! converter records an explicit reinstall/re-auth requirement and writes no
//! manifest or installation state. Unknown executable artifacts are never
//! silently made live.

use std::sync::Arc;

use ironclaw::channels::wasm::{StoredWasmChannel, WasmChannelStore};
use ironclaw::tools::wasm::{StoredWasmTool, WasmToolStore};

use crate::error::MigrationError;
use crate::options::MigrationOptions;
use crate::report::{Domain, LossReason, MigrationReport};
use crate::source::V1Source;
use crate::target::RebornTarget;

pub(crate) async fn run(
    src: &V1Source,
    _tgt: &mut RebornTarget,
    _options: &MigrationOptions,
    report: &mut MigrationReport,
) -> Result<(), MigrationError> {
    let tool_store = build_tool_store(src);
    let channel_store = build_channel_store(src);

    let mut users: std::collections::BTreeSet<String> =
        src.distinct_users().await?.into_iter().collect();
    users.extend(src.distinct_user_ids_in("wasm_tools", "user_id").await?);
    users.extend(src.distinct_user_ids_in("wasm_channels", "user_id").await?);

    for user in users {
        if let Some(store) = tool_store.as_ref() {
            for tool in store
                .list(&user)
                .await
                .map_err(|error| MigrationError::ReadSource {
                    domain: "wasm_tools".into(),
                    reason: error.to_string(),
                })?
            {
                record_tool_disposition(store.as_ref(), &tool, report).await?;
            }
        }

        if let Some(store) = channel_store.as_ref() {
            for channel in store
                .list(&user)
                .await
                .map_err(|error| MigrationError::ReadSource {
                    domain: "wasm_channels".into(),
                    reason: error.to_string(),
                })?
            {
                record_channel_disposition(&channel, report);
            }
        }
    }
    Ok(())
}

async fn record_tool_disposition(
    store: &dyn WasmToolStore,
    tool: &StoredWasmTool,
    report: &mut MigrationReport,
) -> Result<(), MigrationError> {
    let source_id = format!("tool:{}", tool.name);
    report.record_loss(
        Domain::Extension,
        &source_id,
        "package",
        LossReason::NoTargetConcept,
        "v1 WASM has no catalog-backed Reborn package; no installation was written or enabled. \
         Reinstall a compatible Reborn extension after cutover"
            .to_string(),
    );

    match store.get_capabilities(tool.id).await {
        Ok(Some(_)) => report.record_loss(
            Domain::Extension,
            source_id,
            "capabilities",
            LossReason::NoTargetField,
            "v1 HTTP/workspace/rate-limit capability policy and allowed-secret bindings are \
             archive-only; review and bind credentials again after reinstall"
                .to_string(),
        ),
        Ok(None) => {}
        Err(error) => {
            return Err(MigrationError::ReadSource {
                domain: "tool_capabilities".into(),
                reason: error.to_string(),
            });
        }
    }
    Ok(())
}

fn record_channel_disposition(channel: &StoredWasmChannel, report: &mut MigrationReport) {
    let source_id = format!("channel:{}", channel.name);
    report.record_loss(
        Domain::Extension,
        &source_id,
        "package",
        LossReason::NoTargetConcept,
        "v1 WASM channel has no catalog-backed Reborn package; no installation was written or \
         enabled. Reinstall a compatible Reborn extension after cutover"
            .to_string(),
    );
    report.record_loss(
        Domain::Extension,
        source_id,
        "credential_binding",
        LossReason::NoTargetField,
        "v1 has no explicit channel-to-secret join; re-authenticate and bind credentials after \
         reinstall"
            .to_string(),
    );
}

fn build_tool_store(src: &V1Source) -> Option<Arc<dyn WasmToolStore>> {
    #[cfg(feature = "libsql")]
    if let Some(db) = src.handles.libsql_db.as_ref() {
        return Some(Arc::new(ironclaw::tools::wasm::LibSqlWasmToolStore::new(
            db.clone(),
        )));
    }
    #[cfg(feature = "postgres")]
    if let Some(pool) = src.handles.pg_pool.as_ref() {
        return Some(Arc::new(ironclaw::tools::wasm::PostgresWasmToolStore::new(
            pool.clone(),
        )));
    }
    None
}

fn build_channel_store(src: &V1Source) -> Option<Arc<dyn WasmChannelStore>> {
    #[cfg(feature = "libsql")]
    if let Some(db) = src.handles.libsql_db.as_ref() {
        return Some(Arc::new(
            ironclaw::channels::wasm::LibSqlWasmChannelStore::new(db.clone()),
        ));
    }
    #[cfg(feature = "postgres")]
    if let Some(pool) = src.handles.pg_pool.as_ref() {
        return Some(Arc::new(
            ironclaw::channels::wasm::PostgresWasmChannelStore::new(pool.clone()),
        ));
    }
    None
}

