# ironclaw_telemetry

The tenant-scoped BI telemetry domain. It owns bounded observation aggregation,
hourly durable record grammar, the non-blocking recorder worker, and bounded
tenant-scoped export reads.

Telemetry persistence uses a typed `FilesystemTelemetryRepository` over the
existing `ScopedFilesystem` mount at `/tenant-shared/telemetry/v0`. The domain
does not choose PostgreSQL, libSQL, or local-disk backends, receive raw
`RootFilesystem` authority, construct physical tenant paths, or expose a
backend-selection repository trait. Ordered projections lead with `tenant_id`
and bounded reads use half-open UTC ranges and keyset cursors.

The crate depends downward on `ironclaw_telemetry_contracts` and
`ironclaw_filesystem`; composition selects and mounts the concrete root
filesystem. It does not depend on product, composition, or any producer.
