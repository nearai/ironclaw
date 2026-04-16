//! Optional platform ClickHouse sink for runtime logs.
//!
//! When `ironclaw` runs in platform-managed mode (`PLATFORM_MANAGED=true`)
//! and a ClickHouse URL is provided, this tracing `Layer` ships runtime log
//! events to the `platform_logs` table via a non-blocking bounded channel.
//!
//! ## Design Decisions
//!
//! - **No lobsterpool dependency**: Replicates the proven `ClickHouseLayer` pattern
//!   from `lobsterpool/crates/lp-server/src/clickhouse_layer.rs` without importing it.
//! - **Fire-and-forget**: Main thread never blocks on ClickHouse writes.
//! - **Bounded queue + backlog cap**: Prevents unbounded memory growth.
//! - **Runtime rebind**: Supports TidePool fast-path — sink can start disabled and
//!   be activated after `/api/configure` injects agent/tenant context.
//! - **No hidden telemetry**: Disabled by default. Only enabled when platform
//!   explicitly injects `IRONCLAW_PLATFORM_CH_URL` and `PLATFORM_MANAGED=true`.
//!
//! All error reporting uses `eprintln!` to avoid infinite recursion through
//! the tracing subscriber.

use std::sync::{Arc, RwLock};

use tokio::sync::mpsc;
use tracing::Subscriber;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

use super::runtime_log::{fields, SpanContext, SpanContextVisitor};

// ── Constants ────────────────────────────────────────────────────────────

/// Maximum consecutive flush retries before discarding a batch.
const MAX_RETRIES: usize = 3;
/// Maximum pending rows in the flush buffer.
const MAX_BACKLOG: usize = 50_000;
/// Retry delays in seconds.
const RETRY_DELAYS_SECS: [u64; 3] = [2, 5, 15];
/// Channel capacity for the bounded log queue.
const CHANNEL_CAPACITY: usize = 10_000;
/// Batch flush threshold.
const BATCH_SIZE: usize = 200;
/// Flush interval in seconds.
const FLUSH_INTERVAL_SECS: u64 = 5;

// ── Env var names ────────────────────────────────────────────────────────

/// ClickHouse URL injected by the platform at container creation or configure time.
pub const ENV_PLATFORM_CH_URL: &str = "IRONCLAW_PLATFORM_CH_URL";
/// Agent ID injected by the platform.
pub const ENV_PLATFORM_AGENT_ID: &str = "IRONCLAW_PLATFORM_AGENT_ID";
/// Tenant ID injected by the platform.
pub const ENV_PLATFORM_TENANT_ID: &str = "IRONCLAW_PLATFORM_TENANT_ID";

// ── Sink context (runtime-rebindable) ────────────────────────────────────

/// Runtime context for the platform sink, updatable after initial startup.
#[derive(Debug, Clone)]
pub struct PlatformSinkContext {
    /// ClickHouse HTTP URL (e.g. `http://clickhouse:8123`).
    pub url: String,
    /// Platform agent ID for log correlation.
    pub agent_id: String,
    /// Platform tenant ID for log correlation.
    pub tenant_id: String,
}

/// Shared, runtime-updatable sink context.
///
/// Uses `std::sync::RwLock` (not tokio) because:
/// - `init_tracing()` runs inside a tokio runtime where `blocking_write()` panics
/// - Lock is only held briefly for reads/writes, never across `.await`
///
/// Starts as `None` (disabled). Set via `bind()` after platform configure.
pub type SinkContextHandle = Arc<RwLock<Option<PlatformSinkContext>>>;

/// Create a new unbound sink context handle.
pub fn new_sink_context() -> SinkContextHandle {
    Arc::new(RwLock::new(None))
}

/// Bind the sink context, activating platform log shipping (async-safe).
pub async fn bind_sink_context(handle: &SinkContextHandle, ctx: PlatformSinkContext) {
    bind_sink_context_sync(handle, ctx);
}

/// Bind the sink context (sync version, safe to call from sync or async code).
pub fn bind_sink_context_sync(handle: &SinkContextHandle, ctx: PlatformSinkContext) {
    let mut guard = handle.write().unwrap_or_else(|e| e.into_inner());
    *guard = Some(ctx);
}

/// Try to resolve sink context from environment variables.
///
/// Returns `None` if `PLATFORM_MANAGED` is not `true` or the CH URL is missing.
pub fn resolve_from_env() -> Option<PlatformSinkContext> {
    let platform_managed = std::env::var("PLATFORM_MANAGED")
        .unwrap_or_default()
        .eq_ignore_ascii_case("true");
    if !platform_managed {
        return None;
    }

    let url = std::env::var(ENV_PLATFORM_CH_URL).ok()?;
    if url.is_empty() {
        return None;
    }

    let agent_id = std::env::var(ENV_PLATFORM_AGENT_ID).unwrap_or_default();
    let tenant_id = std::env::var(ENV_PLATFORM_TENANT_ID).unwrap_or_default();

    Some(PlatformSinkContext {
        url,
        agent_id,
        tenant_id,
    })
}

// ── Log event ────────────────────────────────────────────────────────────

struct LogEvent {
    timestamp: String,
    level: String,
    target: String,
    message: String,
    fields: String,
    // Correlation fields from span scope
    request_id: Option<String>,
    channel: Option<String>,
    thread_id: Option<String>,
    job_id: Option<String>,
    session_id: Option<String>,
}

// ── Tracing Layer ────────────────────────────────────────────────────────

/// A tracing [`Layer`] that ships log events to ClickHouse `platform_logs`.
///
/// Disabled by default. Only sends events when `sink_context` is bound
/// (i.e., when the platform has injected CH URL and agent context).
#[derive(Clone)]
pub struct PlatformClickHouseLayer {
    tx: mpsc::Sender<LogEvent>,
    sink_context: SinkContextHandle,
}

impl PlatformClickHouseLayer {
    /// Create a new layer and spawn the background flusher.
    ///
    /// The layer starts in disabled mode — events are only shipped after
    /// the sink context is bound via `bind_sink_context()`.
    pub fn new(sink_context: SinkContextHandle) -> Self {
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        let ctx_handle = Arc::clone(&sink_context);
        tokio::spawn(flush_loop(rx, ctx_handle));
        Self { tx, sink_context }
    }

    /// Returns a handle to the sink context for runtime rebinding.
    pub fn context_handle(&self) -> SinkContextHandle {
        Arc::clone(&self.sink_context)
    }
}

impl<S> Layer<S> for PlatformClickHouseLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::span::Id,
        ctx: Context<'_, S>,
    ) {
        let span = ctx.span(id).expect("span not found, this is a bug");
        let mut span_ctx = SpanContext::default();
        attrs.record(&mut SpanContextVisitor(&mut span_ctx));
        span.extensions_mut().insert(span_ctx);
    }

    fn on_record(
        &self,
        id: &tracing::span::Id,
        values: &tracing::span::Record<'_>,
        ctx: Context<'_, S>,
    ) {
        if let Some(span) = ctx.span(id) {
            let mut extensions = span.extensions_mut();
            if let Some(span_ctx) = extensions.get_mut::<SpanContext>() {
                values.record(&mut SpanContextVisitor(span_ctx));
            }
        }
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);

        let meta = event.metadata();
        let now = chrono::Utc::now();

        // Extract well-known correlation fields from event-level fields.
        // Worker/job/agent lifecycle logs use event fields (not span fields),
        // so we must check both sources to populate platform_logs columns.
        let mut event_ctx = SpanContext::default();
        let mut remaining_fields: Vec<(String, String)> = Vec::new();
        for (k, v) in visitor.other.drain(..) {
            match k.as_str() {
                n if n == fields::REQUEST_ID && event_ctx.request_id.is_none() => {
                    event_ctx.request_id = Some(v);
                }
                n if n == fields::CHANNEL && event_ctx.channel.is_none() => {
                    event_ctx.channel = Some(v);
                }
                n if n == fields::THREAD_ID && event_ctx.thread_id.is_none() => {
                    event_ctx.thread_id = Some(v);
                }
                n if n == fields::JOB_ID && event_ctx.job_id.is_none() => {
                    event_ctx.job_id = Some(v);
                }
                n if n == fields::SESSION_ID && event_ctx.session_id.is_none() => {
                    event_ctx.session_id = Some(v);
                }
                n if n == fields::TENANT_ID && event_ctx.tenant_id.is_none() => {
                    event_ctx.tenant_id = Some(v);
                }
                n if n == fields::AGENT_ID && event_ctx.agent_id.is_none() => {
                    event_ctx.agent_id = Some(v);
                }
                _ => {
                    remaining_fields.push((k, v));
                }
            }
        }

        let fields_json = if remaining_fields.is_empty() {
            String::new()
        } else {
            let map: serde_json::Map<String, serde_json::Value> = remaining_fields
                .into_iter()
                .map(|(k, v)| (k, serde_json::Value::String(v)))
                .collect();
            serde_json::to_string(&serde_json::Value::Object(map)).unwrap_or_default()
        };

        // Extract correlation from span scope. Span fields take precedence
        // only where the event didn't already supply a value.
        let mut span_ctx = event_ctx;
        if let Some(scope) = ctx.event_scope(event) {
            for span in scope {
                let extensions = span.extensions();
                if let Some(sf) = extensions.get::<SpanContext>() {
                    if span_ctx.request_id.is_none() {
                        span_ctx.request_id.clone_from(&sf.request_id);
                    }
                    if span_ctx.channel.is_none() {
                        span_ctx.channel.clone_from(&sf.channel);
                    }
                    if span_ctx.thread_id.is_none() {
                        span_ctx.thread_id.clone_from(&sf.thread_id);
                    }
                    if span_ctx.job_id.is_none() {
                        span_ctx.job_id.clone_from(&sf.job_id);
                    }
                    if span_ctx.session_id.is_none() {
                        span_ctx.session_id.clone_from(&sf.session_id);
                    }
                    if span_ctx.tenant_id.is_none() {
                        span_ctx.tenant_id.clone_from(&sf.tenant_id);
                    }
                    if span_ctx.agent_id.is_none() {
                        span_ctx.agent_id.clone_from(&sf.agent_id);
                    }
                }
                if span_ctx.request_id.is_some()
                    && span_ctx.channel.is_some()
                    && span_ctx.thread_id.is_some()
                    && span_ctx.job_id.is_some()
                    && span_ctx.session_id.is_some()
                    && span_ctx.tenant_id.is_some()
                    && span_ctx.agent_id.is_some()
                {
                    break;
                }
            }
        }

        let log = LogEvent {
            timestamp: now.format("%Y-%m-%d %H:%M:%S%.3f").to_string(),
            level: meta.level().to_string().to_lowercase(),
            target: meta.target().to_string(),
            message: visitor.message,
            fields: fields_json,
            request_id: span_ctx.request_id,
            channel: span_ctx.channel,
            thread_id: span_ctx.thread_id,
            job_id: span_ctx.job_id,
            session_id: span_ctx.session_id,
        };

        // Fire-and-forget — never block the caller.
        if self.tx.try_send(log).is_err() {
            // Channel full — drop the event with evidence. Use eprintln
            // to avoid tracing recursion.
            eprintln!("ironclaw: platform_logs channel full, dropping log event");
        }
    }
}

// ── Field extraction (event-level) ───────────────────────────────────────

#[derive(Default)]
struct FieldVisitor {
    message: String,
    other: Vec<(String, String)>,
}

impl tracing::field::Visit for FieldVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{:?}", value);
        } else {
            self.other
                .push((field.name().to_string(), format!("{:?}", value)));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.other
                .push((field.name().to_string(), value.to_string()));
        }
    }
}

// ── Background flush loop ────────────────────────────────────────────────

async fn flush_loop(mut rx: mpsc::Receiver<LogEvent>, ctx_handle: SinkContextHandle) {
    use tokio::time::{Duration, interval};

    let mut buf: Vec<LogEvent> = Vec::with_capacity(256);
    let mut tick = interval(Duration::from_secs(FLUSH_INTERVAL_SECS));
    // Track whether the sink was previously unbound so we can drain
    // pre-bind events when transitioning to bound (prevents misattribution
    // of warm-pool standby logs to the first claimed agent).
    let mut was_unbound = true;

    loop {
        tokio::select! {
            _ = tick.tick() => {
                if !buf.is_empty() {
                    flush_if_bound(&ctx_handle, &mut buf, &mut was_unbound).await;
                }
            }
            msg = rx.recv() => {
                match msg {
                    Some(event) => {
                        buf.push(event);
                        // Enforce backlog cap.
                        if buf.len() > MAX_BACKLOG {
                            let excess = buf.len() - MAX_BACKLOG;
                            buf.drain(..excess);
                            eprintln!(
                                "ironclaw platform_logs backlog exceeded {MAX_BACKLOG}, dropped {excess} oldest rows"
                            );
                        }
                        if buf.len() >= BATCH_SIZE {
                            flush_if_bound(&ctx_handle, &mut buf, &mut was_unbound).await;
                        }
                    }
                    None => {
                        // Channel closed — flush remaining and exit.
                        if !buf.is_empty() {
                            flush_if_bound(&ctx_handle, &mut buf, &mut was_unbound).await;
                        }
                        break;
                    }
                }
            }
        }
    }
}

/// Flush buffered events if the sink context is bound.
/// If unbound (warm pool / disabled), events accumulate until bound or backlog-capped.
///
/// On the transition from unbound→bound, drains all pre-bind events to prevent
/// warm-pool standby logs from being misattributed to the first claimed agent.
async fn flush_if_bound(
    ctx_handle: &SinkContextHandle,
    buf: &mut Vec<LogEvent>,
    was_unbound: &mut bool,
) {
    let ctx = {
        let guard = ctx_handle.read().unwrap_or_else(|e| e.into_inner());
        guard.clone()
    };

    let Some(ctx) = ctx else {
        // Sink not bound yet — keep buffering (backlog cap handles overflow).
        return;
    };

    // Transition from unbound→bound: discard pre-bind events to prevent
    // warm-pool standby logs from being attributed to the first agent.
    if *was_unbound {
        *was_unbound = false;
        if !buf.is_empty() {
            eprintln!(
                "ironclaw: platform sink bound, discarding {} pre-bind buffered events",
                buf.len()
            );
            buf.clear();
        }
        return;
    }

    flush_with_retry(&ctx, buf).await;
}

/// Attempt to flush `buf` to ClickHouse with bounded retries.
async fn flush_with_retry(ctx: &PlatformSinkContext, buf: &mut Vec<LogEvent>) {
    use tokio::time::{Duration, sleep};

    let body = build_insert_body(ctx, buf);
    if body.is_empty() {
        buf.clear();
        return;
    }

    let insert_url = format!(
        "{}/?query=INSERT+INTO+platform_logs+FORMAT+JSONEachRow",
        ctx.url.trim_end_matches('/')
    );

    let client = reqwest::Client::new();

    for attempt in 0..MAX_RETRIES {
        match client
            .post(&insert_url)
            .header("Content-Type", "application/json")
            .body(body.clone())
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                buf.clear();
                return;
            }
            Ok(resp) => {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                eprintln!(
                    "ironclaw platform_logs flush failed (attempt {}/{}): HTTP {} — {}",
                    attempt + 1,
                    MAX_RETRIES,
                    status,
                    text.chars().take(200).collect::<String>(),
                );
            }
            Err(e) => {
                eprintln!(
                    "ironclaw platform_logs flush failed (attempt {}/{}): {}",
                    attempt + 1,
                    MAX_RETRIES,
                    e,
                );
            }
        }
        if attempt + 1 < MAX_RETRIES {
            sleep(Duration::from_secs(RETRY_DELAYS_SECS[attempt])).await;
        }
    }

    // All retries exhausted — discard batch.
    eprintln!(
        "ironclaw platform_logs flush failed after {} retries, discarding {} rows",
        MAX_RETRIES,
        buf.len()
    );
    buf.clear();
}

/// Build the JSONEachRow body for ClickHouse INSERT.
fn build_insert_body(ctx: &PlatformSinkContext, events: &[LogEvent]) -> String {
    let mut body = String::new();
    for e in events {
        // Build a JSON object for each row.
        let mut row = serde_json::Map::new();
        row.insert(
            "timestamp".to_string(),
            serde_json::Value::String(e.timestamp.clone()),
        );
        row.insert(
            "level".to_string(),
            serde_json::Value::String(e.level.clone()),
        );
        row.insert(
            "target".to_string(),
            serde_json::Value::String(e.target.clone()),
        );
        row.insert(
            "message".to_string(),
            serde_json::Value::String(e.message.clone()),
        );
        row.insert(
            "fields".to_string(),
            serde_json::Value::String(e.fields.clone()),
        );

        // Correlation fields: prefer span-scope, fall back to platform context.
        if let Some(ref rid) = e.request_id {
            row.insert(
                "request_id".to_string(),
                serde_json::Value::String(rid.clone()),
            );
        }
        let agent_id = if !ctx.agent_id.is_empty() {
            ctx.agent_id.clone()
        } else {
            String::new()
        };
        if !agent_id.is_empty() {
            row.insert(
                "agent_id".to_string(),
                serde_json::Value::String(agent_id),
            );
        }
        let tenant_id = if !ctx.tenant_id.is_empty() {
            ctx.tenant_id.clone()
        } else {
            String::new()
        };
        if !tenant_id.is_empty() {
            row.insert(
                "tenant_id".to_string(),
                serde_json::Value::String(tenant_id),
            );
        }

        // New correlation columns from ironclaw runtime.
        if let Some(ref ch) = e.channel {
            row.insert(
                "channel".to_string(),
                serde_json::Value::String(ch.clone()),
            );
        }
        if let Some(ref tid) = e.thread_id {
            row.insert(
                "thread_id".to_string(),
                serde_json::Value::String(tid.clone()),
            );
        }
        if let Some(ref jid) = e.job_id {
            row.insert(
                "job_id".to_string(),
                serde_json::Value::String(jid.clone()),
            );
        }
        if let Some(ref sid) = e.session_id {
            row.insert(
                "session_id".to_string(),
                serde_json::Value::String(sid.clone()),
            );
        }

        if let Ok(json) = serde_json::to_string(&serde_json::Value::Object(row)) {
            body.push_str(&json);
            body.push('\n');
        }
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_from_env_disabled_by_default() {
        // PLATFORM_MANAGED is not set in test env
        assert!(resolve_from_env().is_none());
    }

    #[test]
    fn build_insert_body_empty_events() {
        let ctx = PlatformSinkContext {
            url: "http://localhost:8123".to_string(),
            agent_id: "agent-1".to_string(),
            tenant_id: "tenant-1".to_string(),
        };
        let body = build_insert_body(&ctx, &[]);
        assert!(body.is_empty());
    }

    #[test]
    fn build_insert_body_single_event() {
        let ctx = PlatformSinkContext {
            url: "http://localhost:8123".to_string(),
            agent_id: "agent-1".to_string(),
            tenant_id: "tenant-1".to_string(),
        };
        let events = vec![LogEvent {
            timestamp: "2024-01-01 00:00:00.000".to_string(),
            level: "info".to_string(),
            target: "ironclaw::test".to_string(),
            message: "hello world".to_string(),
            fields: "{}".to_string(),
            request_id: Some("req-123".to_string()),
            channel: Some("gateway".to_string()),
            thread_id: None,
            job_id: None,
            session_id: None,
        }];
        let body = build_insert_body(&ctx, &events);
        assert!(body.contains("\"agent_id\":\"agent-1\""));
        assert!(body.contains("\"tenant_id\":\"tenant-1\""));
        assert!(body.contains("\"request_id\":\"req-123\""));
        assert!(body.contains("\"channel\":\"gateway\""));
        assert!(body.contains("\"message\":\"hello world\""));
        // Each row ends with newline
        assert!(body.ends_with('\n'));
    }

    #[test]
    fn build_insert_body_omits_empty_fields() {
        let ctx = PlatformSinkContext {
            url: "http://localhost:8123".to_string(),
            agent_id: String::new(), // empty — should be omitted
            tenant_id: "tenant-1".to_string(),
        };
        let events = vec![LogEvent {
            timestamp: "2024-01-01 00:00:00.000".to_string(),
            level: "warn".to_string(),
            target: "test".to_string(),
            message: "test".to_string(),
            fields: String::new(),
            request_id: None,
            channel: None,
            thread_id: None,
            job_id: None,
            session_id: None,
        }];
        let body = build_insert_body(&ctx, &events);
        assert!(!body.contains("\"agent_id\""));
        assert!(!body.contains("\"request_id\""));
        assert!(!body.contains("\"channel\""));
        assert!(body.contains("\"tenant_id\":\"tenant-1\""));
    }

    #[test]
    fn sink_context_starts_unbound() {
        let handle = new_sink_context();
        let guard = handle.read().unwrap();
        assert!(guard.is_none());
    }

    #[test]
    fn sink_context_can_be_bound_sync() {
        let handle = new_sink_context();
        bind_sink_context_sync(
            &handle,
            PlatformSinkContext {
                url: "http://ch:8123".to_string(),
                agent_id: "a1".to_string(),
                tenant_id: "t1".to_string(),
            },
        );
        let guard = handle.read().unwrap();
        assert!(guard.is_some());
        let ctx = guard.as_ref().expect("just bound");
        assert_eq!(ctx.agent_id, "a1");
    }
}
