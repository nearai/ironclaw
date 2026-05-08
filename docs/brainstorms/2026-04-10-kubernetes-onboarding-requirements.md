---
date: 2026-04-10
topic: kubernetes-onboarding
---

# Kubernetes Onboarding in Setup Wizard

## Problem Frame

The setup wizard's Step 8 (Container Sandbox) has a full Docker configuration
flow -- runtime detection, image validation, retry on failure -- but the
Kubernetes path is a bare connectivity check that enables or disables the
sandbox with no configuration. When both features are compiled in, the user
has no way to choose Kubernetes interactively; the wizard always runs the
Docker path. Users deploying IronClaw on Kubernetes clusters get no guided
setup for namespace or cluster connectivity.

## Requirements

**Runtime Selection**

- R1. When both `docker` and `kubernetes` features are compiled in, Step 8
  presents a single-select menu: "Docker" / "Kubernetes" (default: Docker).
- R2. When only one runtime feature is compiled in, skip the menu and use
  that runtime directly (current behavior for Docker-only; extend to
  Kubernetes-only).
- R3. The selected runtime is persisted to the database settings table
  as `sandbox.container_runtime` ("docker" or "kubernetes").
- R4. `resolve_runtime_backend()` checks the DB-persisted setting when
  `CONTAINER_RUNTIME` env var is not set. Env var takes precedence over
  the DB setting.

**Kubernetes Configuration**

- R5. When Kubernetes is selected, prompt for namespace with a default:
  `Namespace [ironclaw]:`. The user can accept the default or enter a
  custom namespace.
- R6. Validate cluster connectivity by calling
  `KubernetesRuntime::connect()` + `is_available()` using the chosen
  namespace.
- R7. On connectivity failure, mirror the Docker UX: print an error
  message, offer "Retry?", if retry fails disable sandbox.
- R8. The chosen namespace is persisted to the database settings table
  as `sandbox.k8s_namespace`.

**Wizard UX**

- R9. The Step 8 introductory text should be runtime-agnostic: describe
  sandboxed execution without mentioning Docker specifically.
- R10. The confirm prompt changes from "Enable Docker sandbox?" to
  "Enable container sandbox?" (or similar runtime-neutral wording).

## Success Criteria

- A user compiling with `--features docker,kubernetes` sees a runtime
  choice menu in Step 8 and can select Kubernetes.
- A user selecting Kubernetes is prompted for namespace and gets a
  connectivity check with retry-or-disable flow.
- The selected runtime and namespace are persisted and used at startup
  without requiring env vars.
- Existing Docker-only onboarding is unchanged.

## Scope Boundaries

- No RBAC validation or permission checks during onboarding.
- No image pull validation for Kubernetes (deferred to first workload).
- No namespace auto-creation.
- No orchestrator service name configuration (env-var only).
- No changes to quick mode (it does not run Step 8).

## Key Decisions

- **Runtime selection is interactive when both features are compiled**:
  Rather than auto-detecting or relying solely on env vars, the wizard
  presents a menu. This makes the choice explicit and discoverable.
- **Persist to DB settings, not bootstrap .env**: The runtime choice is
  needed after DB connection (Step 8 runs after Step 1). Env var remains
  as the override mechanism for users who prefer config-as-code.
- **Mirror Docker retry UX for Kubernetes failures**: Keeps the wizard
  experience consistent across backends.
- **No image pull check for Kubernetes**: Unlike Docker where
  `ensure_worker_image()` can pull locally, Kubernetes image pulls depend
  on registry auth and node-level config. Deferring avoids false negatives
  during setup.

## Dependencies / Assumptions

- `SandboxSettings` struct will need new fields: `container_runtime`
  (Option<String>) and `k8s_namespace` (Option<String>).
- `to_db_map()` / `from_db_map()` serialization must be extended.
- `resolve_runtime_backend()` will need a way to receive the DB-persisted
  setting (currently reads only env var). This may require a parameter or
  a second resolution function that accepts a settings reference.

## Outstanding Questions

### Deferred to Planning

- [Affects R4][Technical] How should the DB-persisted runtime setting reach
  `resolve_runtime_backend()`? Options: pass as parameter, add a
  `resolve_with_settings()` variant, or set a process-level env var from
  the loaded setting at startup.
- [Affects R8][Technical] Should `IRONCLAW_K8S_NAMESPACE` env var override
  the DB-persisted namespace, or should they be independent?

## Next Steps

→ `/prompts:ce-plan` for structured implementation planning
