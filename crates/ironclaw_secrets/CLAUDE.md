# ironclaw_secrets guardrails

- Own scoped secret metadata, storage, and one-shot lease mechanics only.
- Never expose raw secret material through metadata, errors, debug output, audit records, events, or dispatch results.
- Preserve tenant/user/project isolation; no global handle lookup unless an explicit admin-scoped API is introduced later.
- Do not implement authorization, approval, run-state, runtime injection, network access, process lifecycle, or product workflow semantics here.
- Keep raw secret access explicit through `SecretStore::consume(...)`; consumers must request a scoped lease first.
