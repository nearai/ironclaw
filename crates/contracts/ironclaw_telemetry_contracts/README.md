# ironclaw_telemetry_contracts

The neutral tenant-telemetry membrane. This contracts-layer crate will own
bounded, provider-neutral observations and the synchronous recorder port used
by canonical producers and the domain-owned buffered implementation.

It has one workspace dependency, `ironclaw_host_api`, for the canonical typed
tenant and user identities. It contains no storage, execution, queue, driver,
product, or transport behavior. The durable domain implementation is
filesystem-backed; this crate remains the provider-neutral observation and
recorder boundary.

See the target-architecture contracts family specification for the telemetry
boundary and the scoped storage placement contract for durable layout.
