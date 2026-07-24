use std::{sync::Arc, time::Instant};

use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    HostApiError, MountAlias, MountGrant, MountPermissions, MountView, ResourceScope, SecretHandle,
    VirtualPath,
};
use ironclaw_secrets::{
    SecretError, SecretMaterial, SecretStore, SecretStoreError, SecretStorePort, SecretsCrypto,
};

use crate::{
    Args, Backend, Sample,
    progress::{ProgressCounters, spawn_progress_reporter, stop_progress_reporter},
    summary::FailureCause,
    synthetic::SyntheticIds,
    trace::{spawn_trace_reporter, stop_trace_reporter},
};

const STRESS_SECRET_HANDLE: &str = "ironclaw_stress_secret";
const STRESS_SECRET_MASTER_KEY: &str = "0123456789abcdef0123456789abcdef";

pub(crate) struct SecretConsumeWorkload {
    store: Arc<dyn SecretStorePort>,
    target: String,
}

impl SecretConsumeWorkload {
    pub(crate) fn target(&self) -> &str {
        &self.target
    }
}

pub(crate) async fn build_secret_consume_workload(
    args: &Args,
    run_id: &str,
) -> Result<SecretConsumeWorkload, String> {
    match args.backend {
        Backend::Libsql => build_libsql_secret_consume_workload(args, run_id).await,
        Backend::Postgres => build_postgres_secret_consume_workload(args, run_id).await,
    }
}

async fn build_libsql_secret_consume_workload(
    args: &Args,
    run_id: &str,
) -> Result<SecretConsumeWorkload, String> {
    let (filesystem, target) = crate::build_libsql_root(args).await?;
    secret_consume_workload_from_root(filesystem, run_id, target)
}

async fn build_postgres_secret_consume_workload(
    args: &Args,
    run_id: &str,
) -> Result<SecretConsumeWorkload, String> {
    let (filesystem, _pool, target) = crate::build_postgres_root_and_pool(args).await?;
    secret_consume_workload_from_root(filesystem, run_id, target)
}

fn secret_consume_workload_from_root<F>(
    root: Arc<F>,
    run_id: &str,
    target: String,
) -> Result<SecretConsumeWorkload, String>
where
    F: RootFilesystem + 'static,
{
    let run_id = run_id.to_string();
    let scoped = Arc::new(ScopedFilesystem::new(root, {
        let run_id = run_id.clone();
        move |scope| secret_mount_view(&run_id, scope)
    }));
    let crypto = Arc::new(
        SecretsCrypto::new(SecretMaterial::from(STRESS_SECRET_MASTER_KEY.to_string()))
            .map_err(secret_crypto_error)?,
    );
    let store: Arc<dyn SecretStorePort> = Arc::new(SecretStore::new(scoped, crypto));
    Ok(SecretConsumeWorkload { store, target })
}

pub(crate) async fn prefill_secrets(
    workload: Arc<SecretConsumeWorkload>,
    args: &Args,
    identities: Arc<SyntheticIds>,
) -> Result<(), String> {
    let handle = stress_secret_handle()?;
    for user_index in 0..args.users {
        let scope = identities.scope_for_user_index(user_index)?;
        workload
            .store
            .put(
                scope,
                handle.clone(),
                SecretMaterial::from(format!("secret-material-{user_index}")),
                None,
            )
            .await
            .map(|_| ())
            .map_err(|error| format!("secret prefill user {user_index}: {error}"))?;
    }
    Ok(())
}

pub(crate) async fn run_secret_consume_tasks(
    workload: Arc<SecretConsumeWorkload>,
    args: &Args,
    identities: Arc<SyntheticIds>,
) -> Result<Vec<Sample>, String> {
    let operation_target = args.operation_target();
    let progress = Arc::new(ProgressCounters::new(args.trace_jsonl.is_some()));
    let progress_reporter = spawn_progress_reporter(
        crate::log_prefix(args),
        args.backend.as_str(),
        args.scenario.as_str(),
        args.progress_interval_seconds,
        operation_target.progress_total(),
        Arc::clone(&progress),
    );
    let trace_reporter = spawn_trace_reporter(args, workload.target(), Arc::clone(&progress));

    let mut tasks = Vec::with_capacity(args.concurrency);
    for worker_index in 0..args.concurrency {
        let workload = Arc::clone(&workload);
        let identities = Arc::clone(&identities);
        let progress = Arc::clone(&progress);
        let args = args.clone();
        tasks.push((
            worker_index,
            tokio::spawn(async move {
                let mut samples = Vec::with_capacity(args.initial_worker_sample_capacity());
                let started = Instant::now();
                let mut operation_index = 0;
                while should_run_operation(args.operation_target(), started, operation_index) {
                    let sample = run_one_secret_consume(
                        Arc::clone(&workload),
                        &args,
                        &identities,
                        worker_index,
                        operation_index,
                    )
                    .await;
                    progress.record(sample.error.is_some(), sample.latency);
                    samples.push(sample);
                    operation_index += 1;
                }
                samples
            }),
        ));
    }

    let mut samples = Vec::with_capacity(operation_target.progress_total().unwrap_or_else(|| {
        args.concurrency
            .saturating_mul(args.initial_worker_sample_capacity())
    }));
    for (worker_index, task) in tasks {
        match task.await {
            Ok(worker_samples) => samples.extend(worker_samples),
            Err(error) => {
                stop_trace_reporter(trace_reporter);
                stop_progress_reporter(progress_reporter);
                return if error.is_panic() {
                    Err(format!("secret-consume worker {worker_index} panicked"))
                } else {
                    Err(format!("secret-consume worker {worker_index} cancelled"))
                };
            }
        }
    }
    stop_trace_reporter(trace_reporter);
    stop_progress_reporter(progress_reporter);

    if let Some(expected) = operation_target.progress_total()
        && samples.len() != expected
    {
        return Err(format!(
            "collected {} samples but expected {expected}",
            samples.len()
        ));
    }
    Ok(samples)
}

async fn run_one_secret_consume(
    workload: Arc<SecretConsumeWorkload>,
    args: &Args,
    identities: &SyntheticIds,
    worker_index: usize,
    operation_index: usize,
) -> Sample {
    let started = Instant::now();
    let outcome =
        run_one_secret_consume_inner(workload, args, identities, worker_index, operation_index)
            .await;
    let latency = started.elapsed();
    let failure = outcome.err();
    let error = failure.as_ref().map(|cause| cause.bucket.clone());
    Sample {
        latency,
        error,
        failure,
        stages: None,
    }
}

async fn run_one_secret_consume_inner(
    workload: Arc<SecretConsumeWorkload>,
    args: &Args,
    identities: &SyntheticIds,
    worker_index: usize,
    operation_index: usize,
) -> Result<(), FailureCause> {
    let scope = identities.scope(args, worker_index, operation_index);
    let handle = stress_secret_handle()
        .map_err(|error| FailureCause::new("invalid_request", "secret_handle", error))?;
    let lease = workload
        .store
        .lease_once(&scope, &handle)
        .await
        .map_err(|error| secret_failure("lease_once", error))?;
    let _material = workload
        .store
        .consume(&scope, lease.id)
        .await
        .map_err(|error| secret_failure("consume", error))?;
    Ok(())
}

fn should_run_operation(
    operation_target: crate::OperationTarget,
    started: Instant,
    operation_index: usize,
) -> bool {
    match operation_target {
        crate::OperationTarget::Fixed {
            operations_per_worker,
            ..
        } => operation_index < operations_per_worker,
        crate::OperationTarget::Duration { duration } => started.elapsed() < duration,
    }
}

fn secret_mount_view(run_id: &str, scope: &ResourceScope) -> Result<MountView, HostApiError> {
    let tenant = scope.tenant_id.as_str();
    let user = scope.user_id.as_str();
    MountView::new(vec![MountGrant::new(
        MountAlias::new("/secrets")?,
        VirtualPath::new(format!(
            "/secrets/stress/{run_id}/tenants/{tenant}/users/{user}/secrets"
        ))?,
        MountPermissions::read_write_list_delete(),
    )])
}

fn stress_secret_handle() -> Result<SecretHandle, String> {
    SecretHandle::new(STRESS_SECRET_HANDLE).map_err(|error| format!("build secret handle: {error}"))
}

fn secret_failure(stage: &'static str, error: SecretStoreError) -> FailureCause {
    let bucket = match &error {
        SecretStoreError::UnknownSecret { .. } => "secret_unknown",
        SecretStoreError::UnknownLease { .. } => "secret_lease_unknown",
        SecretStoreError::LeaseConsumed { .. } => "secret_lease_consumed",
        SecretStoreError::LeaseRevoked { .. } => "secret_lease_revoked",
        SecretStoreError::LeaseExpired { .. } | SecretStoreError::SecretExpired => "secret_expired",
        SecretStoreError::BackendMisconfigured { .. } => "secret_backend_misconfigured",
        SecretStoreError::StoreUnavailable { .. } => "secret_store_unavailable",
    };
    FailureCause::new(bucket, stage, error)
}

fn secret_crypto_error(error: SecretError) -> String {
    format!("secret crypto setup failed: {error}")
}
