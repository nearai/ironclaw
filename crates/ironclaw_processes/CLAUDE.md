# ironclaw_processes guardrails

- Own process lifecycle, process state stores, result/output stores, cancellation, subscriptions, and process services.
- Do not own authorization, approval resolution, capability invocation workflow, dispatcher routing, or product workflows.
- Process records must not persist raw input or runtime output; output belongs in `ProcessResultRecord`/output refs.
- Lifecycle operations are tenant/user scoped and must not reveal cross-tenant existence.
- Resource reservation ownership/cleanup is store-managed; callers cannot forge reservation handles.
