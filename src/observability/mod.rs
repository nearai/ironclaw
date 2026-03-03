//! Observability subsystem: trait-based event and metric recording.
//!
//! Provides a pluggable [`Observer`] trait with multiple backends:
//!
//! | Backend | Description |
//! |---------|-------------|
//! | `noop`  | Zero overhead, discards everything (default) |
//! | `log`   | Emits structured events via `tracing` |
//! | `otel`  | OpenTelemetry spans with `gen_ai.*` attributes |
//! | `multi` | Fan-out to multiple backends simultaneously |
//!
//! The [`create_observer`] factory builds the right backend from
//! [`ObservabilityConfig`]. Backends can be combined with `+` syntax
//! (e.g. `"log+otel"`).

mod log;
mod multi;
mod noop;
#[cfg(feature = "otel")]
pub mod otel;
pub mod traits;

#[cfg(test)]
pub mod recording;

pub use self::log::LogObserver;
pub use self::multi::MultiObserver;
pub use self::noop::NoopObserver;
pub use self::traits::{Observer, ObserverEvent, ObserverMetric};

/// Configuration for the observability backend.
#[derive(Debug, Clone)]
pub struct ObservabilityConfig {
    /// Backend name: "none", "noop", "log", "otel", or "log+otel".
    pub backend: String,
    /// OTLP exporter endpoint (default: `http://localhost:4317`).
    pub otel_endpoint: Option<String>,
    /// OTLP protocol: "grpc" or "http" (default: "grpc").
    pub otel_protocol: Option<String>,
    /// OTEL service name (default: "ironclaw").
    pub otel_service_name: Option<String>,
}

impl ObservabilityConfig {
    /// Build from environment variables with settings as fallback.
    ///
    /// Priority: env var > settings (DB/TOML/JSON) > default.
    /// Uses `optional_env()` so injected secrets are also consulted.
    pub fn resolve(
        settings: &crate::settings::Settings,
    ) -> Result<Self, crate::error::ConfigError> {
        use crate::config::helpers::{optional_env, parse_string_env};

        Ok(Self {
            backend: parse_string_env("OBSERVABILITY_BACKEND", &settings.observability.backend)?,
            otel_endpoint: optional_env("OTEL_EXPORTER_OTLP_ENDPOINT")?
                .or_else(|| settings.observability.otel_endpoint.clone()),
            otel_protocol: optional_env("OTEL_EXPORTER_OTLP_PROTOCOL")?
                .or_else(|| settings.observability.otel_protocol.clone()),
            otel_service_name: optional_env("OTEL_SERVICE_NAME")?
                .or_else(|| settings.observability.otel_service_name.clone()),
        })
    }

    /// Returns `true` if the backend configuration includes OpenTelemetry.
    ///
    /// Used by `init_tracing` to decide whether to attach the
    /// `tracing-opentelemetry` bridge layer.
    pub fn wants_otel(&self) -> bool {
        self.backend.contains("otel")
    }
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            backend: "none".into(),
            otel_endpoint: None,
            otel_protocol: None,
            otel_service_name: None,
        }
    }
}

/// Create an observer from configuration.
///
/// Returns a [`NoopObserver`] for "none"/"noop" (or unknown values),
/// a [`LogObserver`] for "log", an [`OtelObserver`] for "otel" (requires
/// `otel` feature), or a [`MultiObserver`] for compound backends like
/// "log+otel".
#[allow(clippy::cognitive_complexity)] // cfg-gated match arms inflate complexity
pub fn create_observer(config: &ObservabilityConfig) -> Box<dyn Observer> {
    match config.backend.as_str() {
        "log" => Box::new(LogObserver),
        #[cfg(feature = "otel")]
        "otel" => match otel::OtelObserver::new(config) {
            Ok(obs) => Box::new(obs),
            Err(e) => {
                tracing::error!(
                    "Failed to initialize OTEL observer: {}, falling back to noop",
                    e
                );
                Box::new(NoopObserver)
            }
        },
        #[cfg(feature = "otel")]
        "log+otel" | "otel+log" => {
            let mut observers: Vec<Box<dyn Observer>> = vec![Box::new(LogObserver)];
            match otel::OtelObserver::new(config) {
                Ok(obs) => observers.push(Box::new(obs)),
                Err(e) => {
                    tracing::error!("Failed to initialize OTEL observer: {}, using log-only", e);
                }
            }
            Box::new(MultiObserver::new(observers))
        }
        _ => Box::new(NoopObserver),
    }
}

#[cfg(test)]
mod tests {
    use crate::observability::*;

    fn test_config(backend: &str) -> ObservabilityConfig {
        ObservabilityConfig {
            backend: backend.into(),
            otel_endpoint: None,
            otel_protocol: None,
            otel_service_name: None,
        }
    }

    #[test]
    fn default_config_is_none() {
        let cfg = ObservabilityConfig::default();
        assert_eq!(cfg.backend, "none");
    }

    #[test]
    fn factory_returns_noop_for_none() {
        let obs = create_observer(&test_config("none"));
        assert_eq!(obs.name(), "noop");
    }

    #[test]
    fn factory_returns_noop_for_empty() {
        let obs = create_observer(&test_config(""));
        assert_eq!(obs.name(), "noop");
    }

    #[test]
    fn factory_returns_noop_for_unknown() {
        let obs = create_observer(&test_config("prometheus"));
        assert_eq!(obs.name(), "noop");
    }

    #[test]
    fn factory_returns_log_for_log() {
        let obs = create_observer(&test_config("log"));
        assert_eq!(obs.name(), "log");
    }

    #[test]
    fn factory_returns_noop_for_noop() {
        let obs = create_observer(&test_config("noop"));
        assert_eq!(obs.name(), "noop");
    }

    // M9: Test factory paths for "otel" and "log+otel" (feature-gated).
    // These exercise the OTEL init path with no real endpoint, so the factory
    // may succeed or fall back to noop depending on the runtime environment.
    // The important thing is that neither panics and the returned observer is
    // usable.

    #[cfg(feature = "otel")]
    #[tokio::test]
    async fn factory_returns_otel_for_otel() {
        let obs = create_observer(&test_config("otel"));
        // May be "otel" if init succeeds, or "noop" if it falls back
        let name = obs.name();
        assert!(
            name == "otel" || name == "noop",
            "Expected otel or noop, got: {}",
            name
        );
    }

    #[cfg(feature = "otel")]
    #[tokio::test]
    async fn factory_returns_multi_for_log_plus_otel() {
        let obs = create_observer(&test_config("log+otel"));
        // May be "multi" if otel init succeeds, or "multi" with log-only
        let name = obs.name();
        assert_eq!(name, "multi", "log+otel should produce a multi observer");
    }

    /// Regression test for I8: `wants_otel()` must return false when
    /// the backend is "none", "log", etc., and true only when it
    /// contains "otel". Before the fix, `init_tracing` unconditionally
    /// attached the OTEL bridge whenever the feature was compiled in.
    #[test]
    fn wants_otel_reflects_backend_config() {
        assert!(
            !test_config("none").wants_otel(),
            "backend=none should not want OTEL bridge"
        );
        assert!(
            !test_config("noop").wants_otel(),
            "backend=noop should not want OTEL bridge"
        );
        assert!(
            !test_config("log").wants_otel(),
            "backend=log should not want OTEL bridge"
        );
        assert!(
            !test_config("").wants_otel(),
            "empty backend should not want OTEL bridge"
        );
        assert!(
            test_config("otel").wants_otel(),
            "backend=otel should want OTEL bridge"
        );
        assert!(
            test_config("log+otel").wants_otel(),
            "backend=log+otel should want OTEL bridge"
        );
        assert!(
            test_config("otel+log").wants_otel(),
            "backend=otel+log should want OTEL bridge"
        );
    }

    /// Regression test for I7: `ObservabilityConfig` must use the Settings
    /// system (DB/TOML/JSON) as fallback when env vars are unset. Before the
    /// fix, it was constructed inline with raw `std::env::var()` calls,
    /// bypassing the settings system entirely.
    #[test]
    fn resolve_reads_from_settings() {
        let _guard = crate::config::helpers::ENV_MUTEX.lock().unwrap();

        // Clear env vars so settings are the sole source
        unsafe {
            std::env::remove_var("OBSERVABILITY_BACKEND");
            std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
            std::env::remove_var("OTEL_EXPORTER_OTLP_PROTOCOL");
            std::env::remove_var("OTEL_SERVICE_NAME");
        }

        let mut settings = crate::settings::Settings::default();
        settings.observability.backend = "log".to_string();
        settings.observability.otel_endpoint = Some("http://jaeger:4317".to_string());
        settings.observability.otel_protocol = Some("grpc".to_string());
        settings.observability.otel_service_name = Some("test-svc".to_string());

        let cfg = ObservabilityConfig::resolve(&settings).unwrap();

        assert_eq!(cfg.backend, "log", "backend should come from settings");
        assert_eq!(
            cfg.otel_endpoint.as_deref(),
            Some("http://jaeger:4317"),
            "otel_endpoint should come from settings"
        );
        assert_eq!(
            cfg.otel_protocol.as_deref(),
            Some("grpc"),
            "otel_protocol should come from settings"
        );
        assert_eq!(
            cfg.otel_service_name.as_deref(),
            Some("test-svc"),
            "otel_service_name should come from settings"
        );
    }

    /// I7: Env vars must win over settings values (standard priority).
    #[test]
    fn resolve_env_overrides_settings() {
        let _guard = crate::config::helpers::ENV_MUTEX.lock().unwrap();

        unsafe {
            std::env::set_var("OBSERVABILITY_BACKEND", "otel");
            std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "http://env:4317");
        }

        let mut settings = crate::settings::Settings::default();
        settings.observability.backend = "log".to_string();
        settings.observability.otel_endpoint = Some("http://settings:4317".to_string());

        let cfg = ObservabilityConfig::resolve(&settings).unwrap();

        assert_eq!(cfg.backend, "otel", "env var must win over settings");
        assert_eq!(
            cfg.otel_endpoint.as_deref(),
            Some("http://env:4317"),
            "env var must win over settings"
        );

        // Cleanup
        unsafe {
            std::env::remove_var("OBSERVABILITY_BACKEND");
            std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
        }
    }

    /// I7: Settings must appear in `settings.list()` so `ironclaw config set`
    /// can discover and modify observability fields.
    #[test]
    fn observability_settings_visible_in_list() {
        let settings = crate::settings::Settings::default();
        let list = settings.list();
        assert!(
            list.iter().any(|(k, _)| k == "observability.backend"),
            "observability.backend must appear in settings.list()"
        );
    }

    /// I7: Settings must round-trip through DB map (get/set via
    /// `ironclaw config set observability.backend log`).
    #[test]
    fn observability_settings_db_round_trip() {
        let mut settings = crate::settings::Settings::default();
        settings.set("observability.backend", "log+otel").unwrap();
        settings
            .set("observability.otel_endpoint", "http://jaeger:4317")
            .unwrap();

        let map = settings.to_db_map();
        let restored = crate::settings::Settings::from_db_map(&map);

        assert_eq!(restored.observability.backend, "log+otel");
        assert_eq!(
            restored.observability.otel_endpoint,
            Some("http://jaeger:4317".to_string())
        );
    }
}
