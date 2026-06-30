# Docker process Sandbox MVP

**Status:** partial MVP implementation slice

This slice adds a Docker-backed compatibility lane for dynamic process execution through Reborn process lifecycle. It is intentionally separate from the manifest-derived `ironclaw_scripts` lane.

## Shape

- `ironclaw_process_sandbox` defines `ProcessSandboxBackend` as the backend-neutral execution contract.
- `ProcessSandboxExecutor` adapts typed `SandboxProcessPlan` JSON from `ProcessExecutionRequest.input` into the configured backend.
- `DockerProcessSandboxBackend` is the first backend implementation.
- The plan describes command intent, container mount aliases, allowed hosts, placeholder environment values, and credential injection targets.
- Physical host paths for `/workspace`, `/ironclaw/state/tools`, and `/ironclaw/state/cache` live in trusted `DockerProcessSandboxConfig`, not in plan JSON.
- Install and run phases are built separately:
  - install phase can use registry/download network and writable tool/cache mounts, but has no credential bindings.
  - run phase requires read-only tool/cache mounts when credentials are present.
- Credentialed runs fail closed unless the plan requires direct egress lockdown and executor config supplies a broker.

## Broker Contract

The MVP includes broker policy mechanics, not the production TLS-intercepting server:

- Sandbox processes receive placeholder values, such as `NOTION_API_KEY`, rather than secret material.
- The broker policy rewrites only approved host/header/placeholder combinations.
- Rewrite audit carries secret aliases and never secret values.
- Redaction helpers remove injected values from response/error text before it is returned through process output.

The production MITM transport still needs to bind this policy to a per-process proxy, generate a per-run CA, and resolve scoped secret leases at request time.

## Docker Image Contract

`Dockerfile.process-sandbox` builds the sibling process image. Its entrypoint:

- installs a mounted broker CA into the container trust store when configured.
- applies `iptables` broker-only egress lockdown when `IRONCLAW_EGRESS_LOCKDOWN=broker-only`.
- drops to the unprivileged `sandbox` user before executing the process command.

The container is ephemeral. Only the trusted workspace/tool/cache mounts persist across runs.

## Follow-Ups

- Wire product approval and secure secret capture into the Reborn process capability path.
- Implement the production HTTPS MITM broker transport using the broker policy core.
- Add Docker integration tests for local lockdown, broker-mediated HTTPS, read-only credentialed mounts, and install cache persistence.
- Add a hosted Kubernetes `AgentSandboxBackend`; `kubernetes-sigs/agent-sandbox` can consume the same `SandboxProcessPlan` through the `ProcessSandboxBackend` trait.
