# ironclaw_storage

Shared storage substrate primitives for Reborn persistence adapters.

## Invariants

- Own DB/storage mechanics only: backend identity, redacted errors, migration descriptors, pagination, serialization helpers, primitive `BlobStore`/`RecordStore` traits, and future append-log/lock/transaction/encrypted-blob primitives.
- Do **not** own domain semantics. No turn/thread/outbound/secret-specific operations or schemas here; domain traits stay in their owning crates.
- Do **not** depend on other IronClaw domain crates. Domain adapters may depend on this crate, not the reverse.
- Errors must stay redacted: no raw SQL/backend messages, host paths, secrets, payload snippets, or provider/runtime details.
- Filesystem remains file-shaped/path authority; this crate is not a universal filesystem replacement.
