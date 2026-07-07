//! Test doubles substituting production ports for the Reborn binary-E2E
//! and host-runtime capability harnesses. One file per substituted port.

mod empty_identity_context_source;
mod fixed_runtime_credential_account_resolver;
mod github_harness_authorizer;
mod harness_capability_port_factory;
mod host_runtime_harness_capability_port_factory;
mod parking_host_runtime;
mod recording_approval_request_store;
mod recording_capability_result_writer;
mod recording_delegating_capability_port;
mod recording_host_runtime;
mod recording_network_http_egress;
mod recording_network_http_transport;
mod recording_runtime_http_egress;
mod recording_test_capability_port;
mod static_capability_surface_profile_resolver;
mod static_network_resolver;
mod static_secret_store;

pub(crate) use empty_identity_context_source::EmptyIdentityContextSource;
pub(crate) use fixed_runtime_credential_account_resolver::FixedRuntimeCredentialAccountResolver;
pub(crate) use github_harness_authorizer::GithubHarnessAuthorizer;
pub(crate) use harness_capability_port_factory::HarnessCapabilityPortFactory;
pub(crate) use host_runtime_harness_capability_port_factory::HostRuntimeHarnessCapabilityPortFactory;
// `ParkingCapabilityGateReleaseGuard` is deliberately not re-exported: tests
// obtain it via `ParkingCapabilityGate::release_guard()` without naming the type.
pub(crate) use parking_host_runtime::{ParkingCapabilityGate, ParkingHostRuntime};
pub(crate) use recording_approval_request_store::RecordingApprovalRequestStore;
pub(crate) use recording_capability_result_writer::RecordingCapabilityResultWriter;
pub(crate) use recording_delegating_capability_port::RecordingDelegatingCapabilityPort;
pub(crate) use recording_host_runtime::RecordingHostRuntime;
pub(crate) use recording_network_http_egress::RecordingNetworkHttpEgress;
pub(crate) use recording_network_http_transport::RecordingNetworkHttpTransport;
pub(crate) use recording_runtime_http_egress::RecordingRuntimeHttpEgress;
// Consts consumed only by the binary-E2E harness in the parity/QA support tree
// (unused in bins that don't mount it).
#[allow(unused_imports)]
pub(crate) use recording_test_capability_port::{
    RecordingTestCapabilityPort, TEST_CAPABILITY_ID, TEST_CAPABILITY_SURFACE_VERSION,
};
pub(crate) use static_capability_surface_profile_resolver::StaticCapabilitySurfaceProfileResolver;
pub(crate) use static_network_resolver::StaticNetworkResolver;
pub(crate) use static_secret_store::StaticSecretStore;
