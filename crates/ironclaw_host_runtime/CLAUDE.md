# ironclaw_host_runtime guardrails

- Own host-side composition shared across Reborn runtime lanes.
- Keep runtime-specific request shapes in the runtime crates; adapters should translate into host API contracts and delegate here.
- Compose low-level services such as `ironclaw_network` and `ironclaw_secrets`; do not duplicate URL parsing, DNS checks, private-IP filtering, HTTP clients, secret stores, or redaction logic in runtime crates.
- Preserve the accounting invariant: `network_egress_bytes` is outbound request bytes only, with response bytes tracked separately.
- Keep raw secret material inside the narrow lease/injection path. Reject runtime-supplied manual credentials, scan raw and percent-decoded URL forms, redact leased values from runtime-visible errors and responses, strip sensitive response headers, and block credential-shaped runtime requests/responses before they reach external services or runtime callers.
- Do not own product workflow, authorization/approval policy, persistence migrations, or event emission unless a later Reborn contract explicitly moves that composition here.
