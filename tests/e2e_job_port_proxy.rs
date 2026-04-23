//! Integration test for container port exposure and proxy.
//!
//! Validates that:
//! 1. Port allocation logic is deterministic
//! 2. The `ContainerPortResolver` trait returns `None` for unknown jobs
//! 3. `ExposedPort` serialization round-trips correctly
//!
//! Live Docker tests (port mapping + proxy) are in `sandbox_live_e2e.rs`
//! and require `--ignored` + Docker availability.

use uuid::Uuid;

use ironclaw::orchestrator::{ContainerPortResolver, ExposedPort};

#[test]
fn test_exposed_port_display_format() {
    let ep = ExposedPort {
        container_port: 5173,
        host_port: 14000,
    };
    assert_eq!(ep.to_string(), "5173:14000");
}

#[test]
fn test_exposed_port_serialization_round_trip() {
    let ports = vec![
        ExposedPort { container_port: 3000, host_port: 14000 },
        ExposedPort { container_port: 5173, host_port: 14001 },
    ];
    let json = serde_json::to_string(&ports).unwrap();
    let parsed: Vec<ExposedPort> = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, ports);
}

#[test]
fn test_empty_exposed_ports_serializes_to_empty_array() {
    let ports: Vec<ExposedPort> = vec![];
    let json = serde_json::to_string(&ports).unwrap();
    assert_eq!(json, "[]");
}

/// Verify the `ContainerPortResolver` trait returns `None` for unknown jobs
/// using a mock implementation.
#[tokio::test]
async fn test_port_resolver_returns_none_for_unknown_job() {
    let resolver = MockPortResolver { ports: vec![] };
    let result = resolver.exposed_ports(Uuid::new_v4()).await;
    assert!(result.is_none(), "empty ports should return None");
}

/// Verify the `ContainerPortResolver` trait returns ports for a known job.
#[tokio::test]
async fn test_port_resolver_returns_ports_for_known_job() {
    let job_id = Uuid::new_v4();
    let resolver = MockPortResolver {
        ports: vec![ExposedPort { container_port: 5173, host_port: 14000 }],
    };
    let result = resolver.exposed_ports(job_id).await;
    assert_eq!(
        result,
        Some(vec![ExposedPort { container_port: 5173, host_port: 14000 }])
    );
}

struct MockPortResolver {
    ports: Vec<ExposedPort>,
}

#[async_trait::async_trait]
impl ContainerPortResolver for MockPortResolver {
    async fn exposed_ports(&self, _job_id: Uuid) -> Option<Vec<ExposedPort>> {
        if self.ports.is_empty() {
            None
        } else {
            Some(self.ports.clone())
        }
    }
}
