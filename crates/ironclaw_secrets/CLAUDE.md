# ironclaw_secrets guardrails

- Own scoped secret metadata, credential mapping metadata, AES-256-GCM/HKDF encryption, encrypted-row repository boundaries, filesystem-backed encrypted persistence, and one-shot lease mechanics only.
- Never expose raw secret material through metadata, errors, debug output, audit records, events, or dispatch results.
- Preserve tenant/user/project isolation; no global handle lookup unless an explicit admin-scoped API is introduced later.
- Do not implement authorization, approval, run-state, runtime injection, network access, process lifecycle, or product workflow semantics here; request-time injection belongs to the host/runtime composition layer after explicit secret access.
- Keep raw secret access explicit through `SecretStore::consume(...)`; consumers must request a scoped lease first.
- Filesystem-backed durability must go through `ironclaw_filesystem::RootFilesystem`; do not add direct PostgreSQL/libSQL adapters or SQL dependencies here.
- This crate may depend on the filesystem abstraction, but it must not depend on workflow/runtime/event/authorization/process crates or concrete runtime crates.
