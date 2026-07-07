use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use reqwest::{Client, Method};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::JoinSet,
};

use crate::{
    Args, Sample,
    progress::ProgressCounters,
    summary::{FailureCause, LatencySummary, latency_summary, summarize_failure_causes},
};

const DEFAULT_READ_MIX: &str = "list_threads=50,timeline=45,session=5";
const MAX_ERROR_BODY_CHARS: usize = 512;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ApiCapacitySummary {
    pub(crate) base_url: String,
    pub(crate) virtual_users: usize,
    pub(crate) message_interval_ms: u64,
    pub(crate) read_qps_per_user: f64,
    pub(crate) read_workers: usize,
    pub(crate) read_mix: String,
    pub(crate) page_size: u32,
    pub(crate) wait_for_assistant: bool,
    pub(crate) terminal_timeout_ms: u64,
    pub(crate) poll_interval_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) mock_llm: Option<MockLlmSummary>,
    pub(crate) endpoints: BTreeMap<String, ApiEndpointSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MockLlmSummary {
    pub(crate) base_url: String,
    pub(crate) model: String,
    pub(crate) latency_ms: u64,
    pub(crate) jitter_ms: u64,
    pub(crate) output_bytes: usize,
    pub(crate) failure_rate: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ApiEndpointSummary {
    pub(crate) attempted: u64,
    pub(crate) succeeded: u64,
    pub(crate) failed: u64,
    pub(crate) throughput_ops_sec: f64,
    pub(crate) latency: LatencySummary,
    pub(crate) errors: BTreeMap<String, u64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) failure_causes: BTreeMap<String, crate::summary::FailureCauseSummary>,
}

#[derive(Debug)]
pub(crate) struct ApiCapacityRun {
    pub(crate) target: String,
    pub(crate) elapsed: Duration,
    pub(crate) samples: Vec<Sample>,
    pub(crate) summary: ApiCapacitySummary,
}

#[derive(Debug, Clone)]
struct ApiUser {
    index: usize,
    label: String,
    bearer_token: Option<String>,
    thread_id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ApiUserInput {
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default, alias = "token")]
    bearer_token: Option<String>,
    #[serde(default, alias = "authorization")]
    authorization: Option<String>,
}

#[derive(Debug, Clone)]
struct ApiIdentity {
    label: String,
    bearer_token: Option<String>,
}

#[derive(Clone)]
struct ApiHarness {
    client: Client,
    base_url: String,
    page_size: u32,
    request_timeout: Duration,
}

#[derive(Debug, Clone)]
struct ApiRequestSample {
    name: &'static str,
    latency: Duration,
    failure: Option<FailureCause>,
}

#[derive(Debug)]
struct ApiCallResult {
    sample: ApiRequestSample,
    value: Result<Value, FailureCause>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadEndpoint {
    ListThreads,
    Timeline,
    Session,
}

#[derive(Debug, Clone)]
struct ReadMixEntry {
    endpoint: ReadEndpoint,
    cumulative_weight: u32,
}

#[derive(Debug, Clone)]
struct ReadMix {
    spec: String,
    total_weight: u32,
    entries: Vec<ReadMixEntry>,
}

#[derive(Debug)]
struct MockLlmHandle {
    summary: MockLlmSummary,
    stop_sender: Option<oneshot::Sender<()>>,
}

impl Drop for MockLlmHandle {
    fn drop(&mut self) {
        if let Some(sender) = self.stop_sender.take() {
            let _ = sender.send(());
        }
    }
}

#[derive(Debug)]
struct MockLlmConfig {
    bind: SocketAddr,
    model: String,
    latency_ms: u64,
    jitter_ms: u64,
    output_bytes: usize,
    failure_rate: f64,
}

struct MockLlmState {
    model: String,
    latency_ms: u64,
    jitter_ms: u64,
    output_bytes: usize,
    failure_rate: f64,
    counter: AtomicU64,
}

pub(crate) fn default_read_mix() -> String {
    DEFAULT_READ_MIX.to_string()
}

pub(crate) fn validate_read_mix(spec: &str) -> Result<(), String> {
    ReadMix::parse(spec).map(|_| ())
}

pub(crate) async fn run(
    args: &Args,
    run_id: &str,
    progress: Arc<ProgressCounters>,
) -> Result<ApiCapacityRun, String> {
    let _mock_llm = start_mock_llm(args).await?;
    let harness = ApiHarness::new(args)?;
    let identities = load_identities(args).await?;
    let users = setup_users(args, run_id, &harness, identities).await?;
    let read_mix = ReadMix::parse(&args.api_read_mix)?;
    let read_workers = read_worker_count(args);

    if let Some(warmup_args) = args.warmup_args() {
        eprintln!(
            "{} warming up target={} duration_seconds={} users={}",
            crate::log_prefix(args),
            harness.base_url,
            warmup_args.duration_seconds,
            users.len()
        );
        let warmup_progress = Arc::new(ProgressCounters::new(false));
        let warmup_namespace = format!("{run_id}:warmup");
        let _ = run_window(
            &warmup_args,
            &warmup_namespace,
            &harness,
            Arc::new(users.clone()),
            &read_mix,
            warmup_progress,
        )
        .await?;
    }

    let started = Instant::now();
    let measured_namespace = format!("{run_id}:measured");
    let window = run_window(
        args,
        &measured_namespace,
        &harness,
        Arc::new(users),
        &read_mix,
        progress,
    )
    .await?;
    let elapsed = started.elapsed();
    let mock_summary = _mock_llm.as_ref().map(|handle| handle.summary.clone());
    let mut endpoints = summarize_api_samples(&window.api_samples, elapsed);
    endpoints.insert(
        "full_flow".to_string(),
        summarize_flow_samples(&window.flow_samples, elapsed),
    );
    let summary = ApiCapacitySummary {
        base_url: harness.base_url.clone(),
        virtual_users: args.users,
        message_interval_ms: args.api_message_interval_ms,
        read_qps_per_user: args.api_read_qps_per_user,
        read_workers,
        read_mix: read_mix.spec,
        page_size: args.api_page_size,
        wait_for_assistant: args.api_wait_for_assistant,
        terminal_timeout_ms: args.api_terminal_timeout_ms,
        poll_interval_ms: args.api_poll_interval_ms,
        mock_llm: mock_summary,
        endpoints,
    };

    Ok(ApiCapacityRun {
        target: harness.base_url.clone(),
        elapsed,
        samples: window.flow_samples,
        summary,
    })
}

struct WindowResult {
    flow_samples: Vec<Sample>,
    api_samples: Vec<ApiRequestSample>,
}

async fn run_window(
    args: &Args,
    operation_namespace: &str,
    harness: &ApiHarness,
    users: Arc<Vec<ApiUser>>,
    read_mix: &ReadMix,
    progress: Arc<ProgressCounters>,
) -> Result<WindowResult, String> {
    let mut tasks = JoinSet::new();
    let stop_reads = Arc::new(tokio::sync::Notify::new());

    for user in users.iter().cloned() {
        let harness = harness.clone();
        let progress = Arc::clone(&progress);
        let args = args.clone();
        let operation_namespace = operation_namespace.to_string();
        tasks.spawn(async move {
            run_virtual_user(&args, &operation_namespace, harness, user, progress).await
        });
    }

    if args.api_read_qps_per_user > 0.0 {
        let read_workers = read_worker_count(args);
        for worker_index in 0..read_workers {
            let harness = harness.clone();
            let users = Arc::clone(&users);
            let read_mix = read_mix.clone();
            let stop_reads = Arc::clone(&stop_reads);
            let args = args.clone();
            tasks.spawn(async move {
                run_read_worker(&args, harness, users, read_mix, worker_index, stop_reads).await
            });
        }
    }

    let mut writer_remaining = users.len();
    let mut flow_samples = Vec::new();
    let mut api_samples = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        let result = joined.map_err(|error| format!("api task join failed: {error}"))??;
        if result.writer_done {
            writer_remaining = writer_remaining.saturating_sub(1);
            if writer_remaining == 0 {
                stop_reads.notify_waiters();
            }
        }
        flow_samples.extend(result.flow_samples);
        api_samples.extend(result.api_samples);
    }

    Ok(WindowResult {
        flow_samples,
        api_samples,
    })
}

struct TaskResult {
    writer_done: bool,
    flow_samples: Vec<Sample>,
    api_samples: Vec<ApiRequestSample>,
}

async fn run_virtual_user(
    args: &Args,
    operation_namespace: &str,
    harness: ApiHarness,
    user: ApiUser,
    progress: Arc<ProgressCounters>,
) -> Result<TaskResult, String> {
    let target = args.operation_target();
    let started = Instant::now();
    let mut operation_index = 0;
    let mut flow_samples = Vec::with_capacity(args.initial_worker_sample_capacity());
    let mut api_samples = Vec::new();

    while crate::should_run_operation(target, started, operation_index) {
        let operation_ref = format!(
            "{operation_namespace}:{}:{}:{}",
            user.label, user.index, operation_index
        );
        let (flow, mut operation_api_samples) =
            run_full_flow(args, &harness, &user, operation_index, &operation_ref).await;
        progress.record(flow.error.is_some(), flow.latency);
        api_samples.append(&mut operation_api_samples);
        flow_samples.push(flow);
        operation_index += 1;
        if args.api_message_interval_ms > 0
            && crate::should_run_operation(target, started, operation_index)
        {
            tokio::time::sleep(Duration::from_millis(args.api_message_interval_ms)).await;
        }
    }

    Ok(TaskResult {
        writer_done: true,
        flow_samples,
        api_samples,
    })
}

async fn run_full_flow(
    args: &Args,
    harness: &ApiHarness,
    user: &ApiUser,
    operation_index: usize,
    operation_ref: &str,
) -> (Sample, Vec<ApiRequestSample>) {
    let started = Instant::now();
    let mut api_samples = Vec::new();
    let body = json!({
        "client_action_id": format!("ironclaw-stress-api:{operation_ref}"),
        "content": stress_payload(
            format!("api stress message {operation_ref}"),
            args.user_message_bytes,
        ),
    });
    let send = harness
        .request_json(
            user,
            "send_message",
            Method::POST,
            &format!("/api/webchat/v2/threads/{}/messages", user.thread_id),
            Some(body),
        )
        .await;
    let send_value = send.value.clone();
    api_samples.push(send.sample);
    let failure = match send_value {
        Ok(value) => {
            if args.api_wait_for_assistant {
                wait_for_assistant(args, harness, user, operation_index, &mut api_samples).await
            } else if submitted_or_already_submitted(&value) {
                None
            } else {
                Some(FailureCause::new(
                    "api_submit_not_accepted",
                    "send_message",
                    compact_json(&value),
                ))
            }
        }
        Err(failure) => Some(failure),
    };

    let latency = started.elapsed();
    let error = failure.as_ref().map(|cause| cause.bucket.clone());
    (
        Sample {
            latency,
            error,
            failure,
            stages: None,
        },
        api_samples,
    )
}

async fn wait_for_assistant(
    args: &Args,
    harness: &ApiHarness,
    user: &ApiUser,
    operation_index: usize,
    api_samples: &mut Vec<ApiRequestSample>,
) -> Option<FailureCause> {
    let deadline = Instant::now() + Duration::from_millis(args.api_terminal_timeout_ms);
    loop {
        let timeline = harness.timeline(user).await;
        let value = timeline.value.clone();
        api_samples.push(timeline.sample);
        match value {
            Ok(value) => {
                if timeline_has_finalized_assistant(&value) {
                    return None;
                }
            }
            Err(failure) => return Some(failure),
        }
        if Instant::now() >= deadline {
            return Some(FailureCause::new(
                "api_full_flow_timeout",
                "timeline",
                format!(
                    "assistant message not visible after {}ms for user={} op={}",
                    args.api_terminal_timeout_ms, user.label, operation_index
                ),
            ));
        }
        tokio::time::sleep(Duration::from_millis(args.api_poll_interval_ms)).await;
    }
}

async fn run_read_worker(
    args: &Args,
    harness: ApiHarness,
    users: Arc<Vec<ApiUser>>,
    read_mix: ReadMix,
    worker_index: usize,
    stop_reads: Arc<tokio::sync::Notify>,
) -> Result<TaskResult, String> {
    let aggregate_qps = args.api_read_qps_per_user * args.users as f64;
    let worker_count = read_worker_count(args).max(1) as f64;
    let sleep = if aggregate_qps <= 0.0 {
        Duration::from_secs(3600)
    } else {
        Duration::from_secs_f64((worker_count / aggregate_qps).max(0.001))
    };
    let mut api_samples = Vec::new();
    let mut operation_index = 0_u64;
    loop {
        tokio::select! {
            _ = stop_reads.notified() => break,
            _ = tokio::time::sleep(sleep) => {}
        }
        let user = &users[(worker_index + operation_index as usize) % users.len()];
        let endpoint = read_mix.endpoint_for(operation_index);
        let call = match endpoint {
            ReadEndpoint::ListThreads => harness.list_threads(user).await,
            ReadEndpoint::Timeline => harness.timeline(user).await,
            ReadEndpoint::Session => harness.session(user).await,
        };
        api_samples.push(call.sample);
        operation_index += 1;
    }

    Ok(TaskResult {
        writer_done: false,
        flow_samples: Vec::new(),
        api_samples,
    })
}

impl ApiHarness {
    fn new(args: &Args) -> Result<Self, String> {
        let base_url = args
            .api_base_url
            .as_ref()
            .ok_or_else(|| {
                "--api-base-url is required for --scenario api-user-capacity".to_string()
            })?
            .trim_end_matches('/')
            .to_string();
        let request_timeout = Duration::from_millis(args.api_request_timeout_ms);
        let client = Client::builder()
            .timeout(request_timeout)
            .build()
            .map_err(|error| format!("build api HTTP client: {error}"))?;
        Ok(Self {
            client,
            base_url,
            page_size: args.api_page_size,
            request_timeout,
        })
    }

    async fn create_thread(
        &self,
        identity: &ApiIdentity,
        requested_thread_id: &str,
    ) -> ApiCallResult {
        let user = ApiUser {
            index: 0,
            label: identity.label.clone(),
            bearer_token: identity.bearer_token.clone(),
            thread_id: requested_thread_id.to_string(),
        };
        self.request_json(
            &user,
            "create_thread",
            Method::POST,
            "/api/webchat/v2/threads",
            Some(json!({
                "client_action_id": format!("ironclaw-stress-api:create:{requested_thread_id}"),
                "requested_thread_id": requested_thread_id,
            })),
        )
        .await
    }

    async fn list_threads(&self, user: &ApiUser) -> ApiCallResult {
        self.request_json(
            user,
            "list_threads",
            Method::GET,
            &format!("/api/webchat/v2/threads?limit={}", self.page_size),
            None,
        )
        .await
    }

    async fn timeline(&self, user: &ApiUser) -> ApiCallResult {
        self.request_json(
            user,
            "timeline",
            Method::GET,
            &format!(
                "/api/webchat/v2/threads/{}/timeline?limit={}",
                user.thread_id, self.page_size
            ),
            None,
        )
        .await
    }

    async fn session(&self, user: &ApiUser) -> ApiCallResult {
        self.request_json(
            user,
            "session",
            Method::GET,
            "/api/webchat/v2/session",
            None,
        )
        .await
    }

    async fn request_json(
        &self,
        user: &ApiUser,
        name: &'static str,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> ApiCallResult {
        let url = format!("{}{}", self.base_url, path);
        let started = Instant::now();
        let mut request = self.client.request(method, &url);
        if let Some(token) = &user.bearer_token {
            request = request.bearer_auth(token);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let result = match tokio::time::timeout(self.request_timeout, request.send()).await {
            Ok(Ok(response)) => {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                if !status.is_success() {
                    Err(FailureCause::new(
                        format!("api_http_status_{}", status.as_u16()),
                        name,
                        truncate_detail(format!("{url}: {text}")),
                    ))
                } else {
                    serde_json::from_str::<Value>(&text).map_err(|error| {
                        FailureCause::new("api_json_decode", name, format!("{url}: {error}"))
                    })
                }
            }
            Ok(Err(error)) => Err(FailureCause::new("api_http_request", name, error)),
            Err(_) => Err(FailureCause::new(
                "api_http_timeout",
                name,
                format!(
                    "{url}: request exceeded {}ms",
                    self.request_timeout.as_millis()
                ),
            )),
        };
        let latency = started.elapsed();
        ApiCallResult {
            sample: ApiRequestSample {
                name,
                latency,
                failure: result.as_ref().err().cloned(),
            },
            value: result,
        }
    }
}

async fn setup_users(
    args: &Args,
    run_id: &str,
    harness: &ApiHarness,
    identities: Vec<ApiIdentity>,
) -> Result<Vec<ApiUser>, String> {
    let setup_concurrency = args.api_setup_concurrency.max(1);
    let mut next_index = 0;
    let mut join_set = JoinSet::new();
    let mut users = Vec::with_capacity(args.users);

    while next_index < args.users || !join_set.is_empty() {
        while next_index < args.users && join_set.len() < setup_concurrency {
            let harness = harness.clone();
            let identity = identities[next_index % identities.len()].clone();
            let thread_id = format!("stress-{run_id}-{next_index}");
            let user_index = next_index;
            join_set.spawn(async move {
                let create = harness.create_thread(&identity, &thread_id).await;
                match create.value {
                    Ok(value) => {
                        let thread_id = extract_thread_id(&value).unwrap_or(thread_id);
                        Ok(ApiUser {
                            index: user_index,
                            label: identity.label,
                            bearer_token: identity.bearer_token,
                            thread_id,
                        })
                    }
                    Err(failure) => Err(format!(
                        "create thread for api user {user_index}: {}: {}",
                        failure.bucket, failure.detail
                    )),
                }
            });
            next_index += 1;
        }
        let Some(joined) = join_set.join_next().await else {
            break;
        };
        users.push(joined.map_err(|error| format!("setup task join failed: {error}"))??);
    }

    users.sort_by_key(|user| user.index);
    Ok(users)
}

async fn load_identities(args: &Args) -> Result<Vec<ApiIdentity>, String> {
    if let Some(path) = &args.api_users_jsonl {
        return load_identity_file(path).await;
    }
    Ok((0..args.users)
        .map(|index| ApiIdentity {
            label: format!("user-{index}"),
            bearer_token: args.api_bearer_token.clone(),
        })
        .collect())
}

async fn load_identity_file(path: &Path) -> Result<Vec<ApiIdentity>, String> {
    let contents = tokio::fs::read_to_string(path)
        .await
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    let mut identities = Vec::new();
    for (line_index, line) in contents.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let input: ApiUserInput = serde_json::from_str(line).map_err(|error| {
            format!("parse {} line {}: {error}", path.display(), line_index + 1)
        })?;
        let bearer_token = input
            .bearer_token
            .or_else(|| input.authorization.and_then(parse_authorization_value));
        identities.push(ApiIdentity {
            label: input
                .user_id
                .unwrap_or_else(|| format!("user-file-{line_index}")),
            bearer_token,
        });
    }
    if identities.is_empty() {
        return Err(format!("{} did not contain any api users", path.display()));
    }
    Ok(identities)
}

fn parse_authorization_value(value: String) -> Option<String> {
    value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
        .map(str::to_string)
        .or(Some(value))
}

fn summarize_api_samples(
    samples: &[ApiRequestSample],
    elapsed: Duration,
) -> BTreeMap<String, ApiEndpointSummary> {
    let mut grouped: BTreeMap<String, Vec<Sample>> = BTreeMap::new();
    for sample in samples {
        grouped
            .entry(sample.name.to_string())
            .or_default()
            .push(Sample {
                latency: sample.latency,
                error: sample
                    .failure
                    .as_ref()
                    .map(|failure| failure.bucket.clone()),
                failure: sample.failure.clone(),
                stages: None,
            });
    }
    let elapsed_secs = elapsed.as_secs_f64().max(f64::MIN_POSITIVE);
    grouped
        .into_iter()
        .map(|(name, samples)| {
            let attempted = samples.len() as u64;
            let failed = samples
                .iter()
                .filter(|sample| sample.error.is_some())
                .count() as u64;
            let succeeded = attempted.saturating_sub(failed);
            let mut errors = BTreeMap::new();
            for error in samples.iter().filter_map(|sample| sample.error.as_ref()) {
                *errors.entry(error.clone()).or_insert(0) += 1;
            }
            let mut latencies: Vec<u128> = samples
                .iter()
                .map(|sample| sample.latency.as_micros())
                .collect();
            latencies.sort_unstable();
            (
                name,
                ApiEndpointSummary {
                    attempted,
                    succeeded,
                    failed,
                    throughput_ops_sec: attempted as f64 / elapsed_secs,
                    latency: latency_summary(&latencies),
                    errors,
                    failure_causes: summarize_failure_causes(&samples),
                },
            )
        })
        .collect()
}

fn summarize_flow_samples(samples: &[Sample], elapsed: Duration) -> ApiEndpointSummary {
    let attempted = samples.len() as u64;
    let failed = samples
        .iter()
        .filter(|sample| sample.error.is_some())
        .count() as u64;
    let succeeded = attempted.saturating_sub(failed);
    let mut errors = BTreeMap::new();
    for error in samples.iter().filter_map(|sample| sample.error.as_ref()) {
        *errors.entry(error.clone()).or_insert(0) += 1;
    }
    let mut latencies: Vec<u128> = samples
        .iter()
        .map(|sample| sample.latency.as_micros())
        .collect();
    latencies.sort_unstable();
    ApiEndpointSummary {
        attempted,
        succeeded,
        failed,
        throughput_ops_sec: attempted as f64 / elapsed.as_secs_f64().max(f64::MIN_POSITIVE),
        latency: latency_summary(&latencies),
        errors,
        failure_causes: summarize_failure_causes(samples),
    }
}

impl ReadMix {
    fn parse(spec: &str) -> Result<Self, String> {
        let mut entries = Vec::new();
        let mut total_weight = 0_u32;
        for raw in spec.split(',') {
            let raw = raw.trim();
            if raw.is_empty() {
                continue;
            }
            let (name, weight) = raw.split_once('=').ok_or_else(|| {
                format!("invalid --api-read-mix entry {raw:?}; expected name=weight")
            })?;
            let weight: u32 = weight
                .parse()
                .map_err(|error| format!("invalid --api-read-mix weight {weight:?}: {error}"))?;
            if weight == 0 {
                continue;
            }
            total_weight = total_weight.saturating_add(weight);
            let endpoint = match name.trim() {
                "list_threads" | "threads" => ReadEndpoint::ListThreads,
                "timeline" => ReadEndpoint::Timeline,
                "session" => ReadEndpoint::Session,
                other => {
                    return Err(format!(
                        "unknown --api-read-mix endpoint {other:?}; expected list_threads, timeline, or session"
                    ));
                }
            };
            entries.push(ReadMixEntry {
                endpoint,
                cumulative_weight: total_weight,
            });
        }
        if entries.is_empty() || total_weight == 0 {
            return Err("--api-read-mix must include at least one positive weight".to_string());
        }
        Ok(Self {
            spec: spec.to_string(),
            total_weight,
            entries,
        })
    }

    fn endpoint_for(&self, operation_index: u64) -> ReadEndpoint {
        let slot = (operation_index % self.total_weight as u64) as u32;
        self.entries
            .iter()
            .find(|entry| slot < entry.cumulative_weight)
            .map(|entry| entry.endpoint)
            .unwrap_or(ReadEndpoint::Timeline)
    }
}

fn read_worker_count(args: &Args) -> usize {
    if args.api_read_workers > 0 {
        args.api_read_workers
    } else {
        args.users.clamp(1, 64)
    }
}

fn extract_thread_id(value: &Value) -> Option<String> {
    value
        .pointer("/thread/thread_id")
        .or_else(|| value.pointer("/thread/id"))
        .or_else(|| value.get("thread_id"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn submitted_or_already_submitted(value: &Value) -> bool {
    matches!(
        value.get("outcome").and_then(Value::as_str),
        Some("submitted" | "already_submitted")
    )
}

fn timeline_has_finalized_assistant(value: &Value) -> bool {
    value
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|message| {
            let kind = message.get("kind").and_then(Value::as_str);
            let status = message.get("status").and_then(Value::as_str);
            kind == Some("assistant") && matches!(status, Some("finalized" | "submitted"))
        })
}

fn stress_payload(prefix: String, minimum_bytes: usize) -> String {
    if minimum_bytes <= prefix.len() {
        return prefix;
    }
    let mut value = prefix;
    value.push(' ');
    while value.len() < minimum_bytes {
        value.push_str("stress-payload ");
    }
    value.truncate(minimum_bytes);
    value
}

fn compact_json(value: &Value) -> String {
    truncate_detail(
        serde_json::to_string(value).unwrap_or_else(|error| format!("json encode error: {error}")),
    )
}

fn truncate_detail(value: impl Into<String>) -> String {
    let value = value.into().replace(['\n', '\r'], " ");
    let mut truncated = value.chars().take(MAX_ERROR_BODY_CHARS).collect::<String>();
    if value.chars().count() > MAX_ERROR_BODY_CHARS {
        truncated.push_str("...");
    }
    truncated
}

async fn start_mock_llm(args: &Args) -> Result<Option<MockLlmHandle>, String> {
    let Some(bind) = args.mock_llm_bind else {
        return Ok(None);
    };
    let config = MockLlmConfig {
        bind,
        model: args.mock_llm_model.clone(),
        latency_ms: args.mock_llm_latency_ms,
        jitter_ms: args.mock_llm_jitter_ms,
        output_bytes: args.mock_llm_output_bytes,
        failure_rate: args.mock_llm_failure_rate,
    };
    let listener = TcpListener::bind(config.bind)
        .await
        .map_err(|error| format!("bind mock llm {}: {error}", config.bind))?;
    let local_addr = listener
        .local_addr()
        .map_err(|error| format!("read mock llm local addr: {error}"))?;
    let state = Arc::new(MockLlmState {
        model: config.model.clone(),
        latency_ms: config.latency_ms,
        jitter_ms: config.jitter_ms,
        output_bytes: config.output_bytes,
        failure_rate: config.failure_rate,
        counter: AtomicU64::new(0),
    });
    let (stop_sender, mut stop_receiver) = oneshot::channel();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut stop_receiver => break,
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, _)) => {
                            let state = Arc::clone(&state);
                            tokio::spawn(async move {
                                let _ = handle_mock_llm_connection(stream, state).await;
                            });
                        }
                        Err(error) => {
                            eprintln!("mock llm accept failed: {error}");
                            break;
                        }
                    }
                }
            }
        }
    });
    let base_url = format!("http://{local_addr}/v1");
    eprintln!(
        "{} mock LLM listening at {base_url} model={}",
        crate::log_prefix(args),
        config.model
    );
    Ok(Some(MockLlmHandle {
        summary: MockLlmSummary {
            base_url,
            model: config.model,
            latency_ms: config.latency_ms,
            jitter_ms: config.jitter_ms,
            output_bytes: config.output_bytes,
            failure_rate: config.failure_rate,
        },
        stop_sender: Some(stop_sender),
    }))
}

async fn handle_mock_llm_connection(
    mut stream: TcpStream,
    state: Arc<MockLlmState>,
) -> Result<(), String> {
    let mut buffer = vec![0_u8; 8192];
    let mut request = Vec::new();
    let header_end = loop {
        let read = stream
            .read(&mut buffer)
            .await
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Ok(());
        }
        request.extend_from_slice(&buffer[..read]);
        if let Some(index) = find_header_end(&request) {
            break index;
        }
        if request.len() > 1024 * 1024 {
            write_mock_response(&mut stream, 413, "text/plain", b"request too large").await?;
            return Ok(());
        }
    };
    let headers = String::from_utf8_lossy(&request[..header_end]).into_owned();
    let content_length = parse_content_length(&headers).unwrap_or(0);
    while request.len() < header_end + 4 + content_length {
        let read = stream
            .read(&mut buffer)
            .await
            .map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
    }
    let body_start = header_end + 4;
    let body = request
        .get(body_start..body_start.saturating_add(content_length))
        .unwrap_or_default();
    let first_line = headers.lines().next().unwrap_or_default();
    let path = first_line.split_whitespace().nth(1).unwrap_or("/");
    match path {
        "/v1/models" | "/models" => {
            let body = json!({
                "object": "list",
                "data": [{"id": state.model, "object": "model"}],
            });
            write_json_response(&mut stream, 200, &body).await
        }
        "/v1/chat/completions" | "/chat/completions" => {
            handle_mock_completion(&mut stream, body, state).await
        }
        _ => write_mock_response(&mut stream, 404, "text/plain", b"not found").await,
    }
}

async fn handle_mock_completion(
    stream: &mut TcpStream,
    body: &[u8],
    state: Arc<MockLlmState>,
) -> Result<(), String> {
    let request_index = state.counter.fetch_add(1, Ordering::Relaxed) + 1;
    if should_fail_mock_request(state.failure_rate, request_index) {
        write_mock_response(
            stream,
            500,
            "application/json",
            br#"{"error":{"message":"mock llm configured failure"}}"#,
        )
        .await?;
        return Ok(());
    }
    let jitter = if state.jitter_ms == 0 {
        0
    } else {
        deterministic_jitter_ms(request_index, state.jitter_ms)
    };
    let wait = state.latency_ms.saturating_add(jitter);
    if wait > 0 {
        tokio::time::sleep(Duration::from_millis(wait)).await;
    }
    let request: Value = serde_json::from_slice(body).unwrap_or_else(|_| json!({}));
    let stream_response = request
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let content = stress_payload("mock assistant response".to_string(), state.output_bytes);
    if stream_response {
        let chunk = json!({
            "id": format!("chatcmpl-stress-{request_index}"),
            "object": "chat.completion.chunk",
            "choices": [{"index": 0, "delta": {"content": content}, "finish_reason": null}]
        });
        let done = json!({
            "id": format!("chatcmpl-stress-{request_index}"),
            "object": "chat.completion.chunk",
            "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]
        });
        let payload = format!("data: {chunk}\n\ndata: {done}\n\ndata: [DONE]\n\n");
        write_mock_response(stream, 200, "text/event-stream", payload.as_bytes()).await
    } else {
        let response = json!({
            "id": format!("chatcmpl-stress-{request_index}"),
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": content},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 16,
                "completion_tokens": 16,
                "total_tokens": 32
            }
        });
        write_json_response(stream, 200, &response).await
    }
}

fn should_fail_mock_request(failure_rate: f64, request_index: u64) -> bool {
    if failure_rate <= 0.0 {
        return false;
    }
    if failure_rate >= 1.0 {
        return true;
    }
    let period = (1.0 / failure_rate).round().max(1.0) as u64;
    request_index.is_multiple_of(period)
}

fn deterministic_jitter_ms(request_index: u64, jitter_ms: u64) -> u64 {
    request_index
        .wrapping_mul(1_103_515_245)
        .wrapping_add(12_345)
        % (jitter_ms + 1)
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_content_length(headers: &str) -> Option<usize> {
    headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse().ok())
            .flatten()
    })
}

async fn write_json_response(
    stream: &mut TcpStream,
    status: u16,
    body: &Value,
) -> Result<(), String> {
    let encoded = serde_json::to_vec(body).map_err(|error| error.to_string())?;
    write_mock_response(stream, status, "application/json", &encoded).await
}

async fn write_mock_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> Result<(), String> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        _ => "OK",
    };
    let headers = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(headers.as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    stream
        .write_all(body)
        .await
        .map_err(|error| error.to_string())?;
    stream.shutdown().await.map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_mix_parses_aliases_and_weights() {
        let mix = ReadMix::parse("threads=2,timeline=3,session=1").unwrap();
        assert_eq!(mix.total_weight, 6);
        assert_eq!(mix.endpoint_for(0), ReadEndpoint::ListThreads);
        assert_eq!(mix.endpoint_for(2), ReadEndpoint::Timeline);
        assert_eq!(mix.endpoint_for(5), ReadEndpoint::Session);
    }

    #[test]
    fn extracts_thread_id_from_create_thread_response() {
        let value = json!({"thread": {"thread_id": "thread-1"}});
        assert_eq!(extract_thread_id(&value).as_deref(), Some("thread-1"));
    }

    #[test]
    fn detects_finalized_assistant_in_timeline() {
        let value = json!({
            "messages": [
                {"kind": "user", "status": "finalized"},
                {"kind": "assistant", "status": "finalized"}
            ]
        });
        assert!(timeline_has_finalized_assistant(&value));
    }
}
