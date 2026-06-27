#!/usr/bin/env python3
"""Hermetic QA matrix runner for Reborn WebUI v2 and OpenAI-compatible rows.

This lane executes local cargo regressions that correspond to QA matrix test
case IDs. It intentionally does not start ``ironclaw-reborn serve`` and does
not call live providers; browser/live coverage belongs in
``scripts/reborn_webui_v2_live_qa``.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shlex
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_OUTPUT_DIR = ROOT / "artifacts" / "reborn-qa-matrix-hermetic"
DEFAULT_TIMEOUT_SECONDS = 45 * 60
PROVIDER = "reborn-qa-matrix"
MODE = "hermetic"


@dataclass(frozen=True)
class CommandSpec:
    name: str
    argv: list[str]
    env: dict[str, str] = field(default_factory=dict)
    unset_env: list[str] = field(default_factory=list)
    description: str = ""


@dataclass(frozen=True)
class CaseSpec:
    name: str
    feature: str
    category: str
    qa_matrix_test_ids: list[str]
    commands: list[CommandSpec]
    default_enabled: bool = True
    notes: str = ""


WEBUI_V2_SERVE_LISTENER_CLI_COMMAND = CommandSpec(
    name="webui_v2_serve_listener_cli_smoke",
    description=(
        "Caller-level ironclaw-reborn serve smoke tests for listener help, "
        "env-bearer fail-closed startup, config seeding before binding, "
        "malformed host rejection, and trusted-laptop host-access listener "
        "guardrails."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_cli",
        "--features",
        "webui-v2-beta",
        "--test",
        "smoke",
        "serve_",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_SERVE_SECURITY_CLI_COMMAND = CommandSpec(
    name="webui_v2_serve_security_cli_smoke",
    description=(
        "Caller-level ironclaw-reborn serve smoke for invalid WebUI security "
        "configuration: canonical host, allowed origins, and max body fallback "
        "must fail closed before listener binding."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_cli",
        "--features",
        "webui-v2-beta",
        "--test",
        "smoke",
        "serve_rejects_invalid_webui_security_config_before_binding",
        "--",
        "--exact",
        "--format",
        "terse",
    ],
)

WEBUI_V2_SERVE_CORS_COMMAND = CommandSpec(
    name="webui_v2_serve_cors_contracts",
    description=(
        "Composed WebUI v2 gateway CORS allow/deny contracts for configured "
        "origins and fail-closed cross-origin preflight behavior."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "webui-v2-beta",
        "--test",
        "webui_v2_serve",
        "cors_",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_SERVE_BODY_LIMIT_COMMAND = CommandSpec(
    name="webui_v2_serve_body_limit_contracts",
    description=(
        "Composed WebUI v2 gateway request-body limit contracts for "
        "descriptor caps and the outer fallback body limit."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "webui-v2-beta",
        "--test",
        "webui_v2_serve",
        "body",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_SERVE_WS_ORIGIN_COMMAND = CommandSpec(
    name="webui_v2_serve_ws_origin_contracts",
    description=(
        "Composed WebUI v2 WebSocket same-origin contracts, including "
        "canonical-host override, missing Origin, and disallowed Origin."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "webui-v2-beta",
        "--test",
        "webui_v2_serve",
        "ws_upgrade_",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_SSO_STARTUP_SMOKE_COMMAND = CommandSpec(
    name="webui_v2_sso_startup_cli_smoke",
    description=(
        "Caller-level ironclaw-reborn serve smoke proving an SSO provider "
        "configured without IRONCLAW_REBORN_WEBUI_ALLOWED_EMAIL_DOMAINS "
        "fails closed before listener binding."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_cli",
        "--features",
        "webui-v2-beta",
        "--test",
        "smoke",
        "serve_fails_closed_when_sso_provider_has_no_allowed_domain_allowlist",
        "--",
        "--exact",
        "--format",
        "terse",
    ],
)

WEBUI_V2_SSO_STARTUP_HELPER_COMMAND = CommandSpec(
    name="webui_v2_sso_startup_helper_contracts",
    description=(
        "serve_sso helper contracts for no-provider behavior, missing "
        "provider secrets, admission allowlist normalization, base URL "
        "precedence, loopback fallback, and public cleartext rejection."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_cli",
        "--features",
        "webui-v2-beta",
        "--bin",
        "ironclaw-reborn",
        "serve_sso",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_SSO_USER_ADMISSION_COMMAND = CommandSpec(
    name="webui_v2_sso_user_admission_contracts",
    description=(
        "WebUI SSO UserDirectory admission contracts for verified allowlisted "
        "canonical/secondary emails, rejection paths, tenant scoping, and "
        "local trigger-access seeding."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_cli",
        "--features",
        "webui-v2-beta",
        "--bin",
        "ironclaw-reborn",
        "user_directory",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_AUTH_SURFACE_COMMAND = CommandSpec(
    name="webui_v2_auth_surface_contracts",
    description=(
        "CLI-owned WebUI auth-surface assembly contracts for env-bearer-only "
        "serve, SSO fail-closed resolver requirements, public login-route "
        "mounting, and local trigger-access bootstrap wiring."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_cli",
        "--features",
        "webui-v2-beta",
        "--bin",
        "ironclaw-reborn",
        "webui_auth",
        "--",
        "--format",
        "terse",
    ],
)


OPENAI_OWNER_CRATE_COMMAND = CommandSpec(
    name="openai_compat_owner_crates",
    description=(
        "Owner-crate regression for Reborn traces, WebUI ingress, "
        "OpenAI-compatible routes/storage, and Slack adapter contracts."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_traces",
        "-p",
        "ironclaw_reborn_webui_ingress",
        "-p",
        "ironclaw_reborn_openai_compat",
        "-p",
        "ironclaw_reborn_openai_compat_storage",
        "-p",
        "ironclaw_slack_v2_adapter",
        "--all-features",
        "--jobs",
        "2",
    ],
)

OPENAI_RESPONSES_WORKFLOW_COMMAND = CommandSpec(
    name="openai_responses_workflow_handlers_contract",
    description=(
        "Focused OpenAI-compatible Responses create, retrieve, cancel, "
        "authorization, validation, idempotency, timeout, and sanitized-error "
        "handler contracts."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_openai_compat",
        "--test",
        "responses_workflow_handlers_contract",
        "--jobs",
        "2",
        "--",
        "--format",
        "terse",
    ],
)

OPENAI_CHAT_WORKFLOW_COMMAND = CommandSpec(
    name="openai_chat_workflow_handlers_contract",
    description=(
        "Focused OpenAI-compatible Chat Completions handler contracts for "
        "success, streaming guardrails, idempotency, validation, projection, "
        "and sanitized error behavior."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_openai_compat",
        "--test",
        "chat_workflow_handlers_contract",
        "--jobs",
        "2",
        "--",
        "--format",
        "terse",
    ],
)

OPENAI_COMPAT_ROUTE_MOUNT_COMMAND = CommandSpec(
    name="openai_compat_beta_route_mount_contracts",
    description=(
        "Focused WebUI v2 composition contracts for OpenAI-compatible beta "
        "protected route mounts, bearer-auth gating, chat/responses ProductWorkflow "
        "submission, and shared turn-admission retention."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "webui-v2-beta,openai-compat-beta,test-support",
        "--test",
        "webui_v2_serve",
        "openai_compat_mount_tests",
        "--",
        "--format",
        "terse",
    ],
)

OPENAI_COMPAT_ALL_FEATURE_COMPOSITION_COMMAND = CommandSpec(
    name="openai_compat_all_feature_composition_contracts",
    description=(
        "All-feature Reborn composition OpenAI-compatible regression under "
        "WebUI v2, OpenAI-compatible, Slack host-beta, and test-support feature "
        "flags."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "webui-v2-beta,openai-compat-beta,slack-v2-host-beta,test-support",
        "openai_compat",
        "--jobs",
        "2",
        "--",
        "--format",
        "terse",
    ],
)

REBORN_COMPOSITION_ALL_FEATURE_COMMAND = CommandSpec(
    name="reborn_composition_all_feature_contracts",
    description=(
        "Full unfiltered ironclaw_reborn_composition regression under the "
        "combined WebUI v2, OpenAI-compatible, Slack host-beta, and "
        "test-support feature set."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    unset_env=["NEARAI_API_KEY", "NEARAI_BASE_URL"],
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "webui-v2-beta,openai-compat-beta,slack-v2-host-beta,test-support",
        "--jobs",
        "2",
        "--",
        "--format",
        "terse",
        "--test-threads=1",
    ],
)

PRODUCT_WORKFLOW_LEDGER_COMMAND = CommandSpec(
    name="product_workflow_storage_durable_ledger",
    description=(
        "Focused durable product-workflow idempotency ledger contract used "
        "below OpenAI-compatible chat completion reservations and replay."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_product_workflow_storage",
        "--test",
        "durable_ledger_contract",
        "--all-features",
        "--jobs",
        "2",
        "--",
        "--format",
        "terse",
    ],
)

REBORN_EVENT_STORE_FOUNDATION_COMMAND = CommandSpec(
    name="reborn_event_store_foundation_contracts",
    description=(
        "Default-feature Reborn foundation crates for config, identity, "
        "event-store, and the runtime facade that back CLI/WebUI audit and "
        "replay behavior without optional live Postgres legs."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_common",
        "-p",
        "ironclaw_reborn_config",
        "-p",
        "ironclaw_reborn_identity",
        "-p",
        "ironclaw_reborn_event_store",
        "-p",
        "ironclaw_reborn",
        "--jobs",
        "2",
        "--",
        "--format",
        "terse",
    ],
)

SUPPORT_SUBSTRATE_COMMAND = CommandSpec(
    name="support_substrate_regression",
    description=(
        "Broad hermetic support-substrate sweep for WebUI v2 attachments, "
        "threads, events/projections/streams, skills, trust, safety, and "
        "product-workflow storage."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_attachments",
        "-p",
        "ironclaw_extractors",
        "-p",
        "ironclaw_events",
        "-p",
        "ironclaw_event_projections",
        "-p",
        "ironclaw_event_streams",
        "-p",
        "ironclaw_prompt_envelope",
        "-p",
        "ironclaw_threads",
        "-p",
        "ironclaw_product_workflow_storage",
        "-p",
        "ironclaw_skills",
        "-p",
        "ironclaw_trust",
        "-p",
        "ironclaw_safety",
        "--all-features",
        "--jobs",
        "2",
        "--",
        "--format",
        "terse",
    ],
)

REBORN_RUNTIME_TOOL_SUBSTRATE_COMMAND = CommandSpec(
    name="reborn_runtime_tool_substrate_contracts",
    description=(
        "Lower runtime/tool crates composed by Reborn WebUI v2 and the "
        "runtime for authorization, policy, network, processes, script/WASM "
        "lanes, extension assets, product context, registry, and loop support."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_authorization",
        "-p",
        "ironclaw_runtime_policy",
        "-p",
        "ironclaw_network",
        "-p",
        "ironclaw_dispatcher",
        "-p",
        "ironclaw_processes",
        "-p",
        "ironclaw_process_sandbox",
        "-p",
        "ironclaw_scripts",
        "-p",
        "ironclaw_wasm",
        "-p",
        "ironclaw_wasm_sandbox_core",
        "-p",
        "ironclaw_wasm_limiter",
        "-p",
        "ironclaw_first_party_extensions",
        "-p",
        "ironclaw_first_party_extension_ports",
        "-p",
        "ironclaw_product_context",
        "-p",
        "ironclaw_product_adapter_registry",
        "-p",
        "ironclaw_loop_support",
        "--all-features",
        "--jobs",
        "2",
        "--",
        "--format",
        "terse",
    ],
)

REBORN_HOOK_BACKEND_ARCHITECTURE_COMMAND = CommandSpec(
    name="reborn_hook_backend_architecture_contracts",
    description=(
        "libSQL hook backend contracts, hook backend parity matrix, and "
        "Reborn architecture boundary checks for route ownership and "
        "durable predicate state behavior."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_hooks_libsql",
        "-p",
        "ironclaw_hooks_parity",
        "-p",
        "ironclaw_architecture",
        "--jobs",
        "2",
        "--",
        "--format",
        "terse",
    ],
)

REBORN_HOOK_POSTGRES_FEATURE_COMMAND = CommandSpec(
    name="reborn_hook_postgres_feature_contracts",
    description=(
        "Postgres-gated hook backend compile/contract/adversarial coverage; "
        "Postgres test bodies guard-skip without a configured database URL."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_hooks_postgres",
        "-p",
        "ironclaw_hooks_parity",
        "--features",
        "postgres",
        "--jobs",
        "2",
        "--",
        "--format",
        "terse",
    ],
)

REBORN_HOOK_POSTGRES_PARITY_INTEGRATION_COMMAND = CommandSpec(
    name="reborn_hook_postgres_parity_integration_contracts",
    description=(
        "Postgres-feature plus integration-gated hook parity coverage; "
        "libSQL multi-host integration remains hermetic and Postgres legs "
        "guard-skip without a configured database URL."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_hooks_parity",
        "--features",
        "postgres,integration",
        "--jobs",
        "2",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_SESSION_SERVICE_SUBSTRATE_COMMAND = CommandSpec(
    name="webui_v2_session_service_substrate_contracts",
    description=(
        "Product workflow and conversation service substrate contracts below "
        "WebUI v2 thread creation, message submission, replay, gates, inbound "
        "adapters, outbound delivery, triggers, projects, and attachment "
        "handling."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_auth",
        "-p",
        "ironclaw_oauth",
        "-p",
        "ironclaw_product_workflow",
        "-p",
        "ironclaw_product_adapters",
        "-p",
        "ironclaw_outbound",
        "-p",
        "ironclaw_triggers",
        "-p",
        "ironclaw_projects",
        "-p",
        "ironclaw_conversations",
        "--all-features",
        "--jobs",
        "2",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_SESSION_EXECUTION_SUBSTRATE_COMMAND = CommandSpec(
    name="webui_v2_session_execution_substrate_contracts",
    description=(
        "Shared execution substrate contracts for WebUI v2 admitted callers: "
        "agent loop, turns, capabilities, approvals, run state, resources, "
        "secrets, memory, filesystem, extensions, MCP, hooks, host API, and "
        "host runtime."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_agent_loop",
        "-p",
        "ironclaw_turns",
        "-p",
        "ironclaw_capabilities",
        "-p",
        "ironclaw_approvals",
        "-p",
        "ironclaw_run_state",
        "-p",
        "ironclaw_resources",
        "-p",
        "ironclaw_secrets",
        "-p",
        "ironclaw_memory",
        "-p",
        "ironclaw_memory_native",
        "-p",
        "ironclaw_filesystem",
        "-p",
        "ironclaw_extensions",
        "-p",
        "ironclaw_mcp",
        "-p",
        "ironclaw_hooks",
        "-p",
        "ironclaw_hooks_libsql",
        "-p",
        "ironclaw_host_api",
        "-p",
        "ironclaw_host_runtime",
        "--all-features",
        "--jobs",
        "2",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_ROUTE_CONTRACT_COMMAND = CommandSpec(
    name="webui_v2_route_contracts",
    description=(
        "Native WebUI v2 route, descriptor, handler, operator, schema, and "
        "SSE capacity contracts."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_webui_v2",
        "--features",
        "webui-v2-beta",
        "--jobs",
        "2",
    ],
)

WEBUI_V2_STATIC_JS_COMMAND = CommandSpec(
    name="webui_v2_static_js_suite",
    description=(
        "Full WebUI v2 static JavaScript node:test discovery suite for "
        "browser-facing SPA modules and client-side API contracts."
    ),
    argv=[
        "bash",
        "-lc",
        (
            "find crates/ironclaw_webui_v2_static/static/js -type f "
            "\\( -name '*test.mjs' -o -name '*test.js' \\) -print0 "
            "| xargs -0 node --test"
        ),
    ],
)

WEBUI_V2_CLIENT_PERSISTENCE_DISCOVERY_COMMAND = CommandSpec(
    name="webui_v2_client_persistence_static_discovery",
    description=(
        "Canonical frontend npm test discovery for WebUI v2 client "
        "persistence/helper contracts, including both .test.js and .test.mjs "
        "static suites."
    ),
    argv=[
        "npm",
        "test",
        "--prefix",
        "crates/ironclaw_webui_v2_static/frontend",
    ],
)

WEBUI_V2_FRONTEND_BUILD_COMMAND = CommandSpec(
    name="webui_v2_frontend_supply_chain_build",
    description=(
        "WebUI v2 frontend supply-chain and bundle build check: install from "
        "the committed package-lock, fail on high-severity npm audit entries, "
        "rebuild static/dist without refreshing vendored CDN assets, and "
        "verify app plus locale chunks exist."
    ),
    argv=[
        "bash",
        "-lc",
        (
            "cd crates/ironclaw_webui_v2_static/frontend "
            "&& npm ci "
            "&& npm audit --audit-level=high "
            "&& bash build.sh --no-vendor "
            "&& test -s ../static/dist/app.js "
            "&& test -n \"$(find ../static/dist/chunks -type f -name '*.js' -print -quit)\""
        ),
    ],
)

WEBUI_V2_STATIC_ROUTER_COMMAND = CommandSpec(
    name="webui_v2_static_router_contracts",
    description=(
        "Focused static-router contracts for SPA shell fallback, known asset "
        "serving, path traversal rejection, asset-like 404s, fresh matching "
        "CSP nonces, locked document CSP allowlists, wallet-connect CSP "
        "isolation, and prefix validation."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_webui_v2_static",
        "--all-features",
        "router",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_WALLET_CONNECT_CLIENT_COMMAND = CommandSpec(
    name="webui_v2_wallet_connect_client_contracts",
    description=(
        "Focused WebUI v2 NEAR wallet connect popup contracts for the fixed "
        "NEAR AI login message/recipient, epoch-millis nonce layout, random "
        "nonce tail, BroadcastChannel success/failure payloads, isolated popup "
        "HTML/importmap, and authenticated app relay to the protected wallet "
        "completion route."
    ),
    argv=[
        "node",
        "--test",
        "crates/ironclaw_webui_v2_static/static/js/lib/wallet-connect-core.test.mjs",
    ],
)

WEBUI_V2_WALLET_CONNECT_ROUTER_COMMAND = CommandSpec(
    name="webui_v2_wallet_connect_static_route",
    description=(
        "Focused WebUI v2 static route contract for the isolated wallet "
        "connect popup's relaxed CSP, no-store cache policy, and strict SPA "
        "shell CSP isolation."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_webui_v2_static",
        "--all-features",
        "wallet_connect_popup_gets_relaxed_csp_and_spa_shell_stays_strict",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_STATIC_AUTH_JS_COMMAND = CommandSpec(
    name="webui_v2_static_auth_js_contract",
    description=(
        "Embedded auth.js contract for login-ticket consumption, URL "
        "credential stripping, stored-token non-overwrite, logout revoke "
        "request dispatch, and OAuth login_error handling."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_webui_v2_static",
        "--all-features",
        "auth_js_carries_login_ticket_contract",
        "--",
        "--exact",
        "--format",
        "terse",
    ],
)

WEBUI_V2_STATIC_API_AUTH_COMMAND = CommandSpec(
    name="webui_v2_static_api_auth_client_contracts",
    description=(
        "Static JS API-client contracts for reading bearer tokens from "
        "sessionStorage, attaching Authorization on same-origin requests, "
        "failing fast on missing ids, rejecting off-origin attachment URLs "
        "before a bearer can be sent, and discovering public OAuth providers "
        "with fail-safe empty-list behavior."
    ),
    argv=[
        "node",
        "--test",
        "crates/ironclaw_webui_v2_static/static/js/lib/api.test.mjs",
    ],
)

WEBUI_V2_LOGIN_OAUTH_CLIENT_COMMAND = CommandSpec(
    name="webui_v2_login_oauth_client_contracts",
    description=(
        "Static login-page OAuth provider contracts for provider ordering, "
        "unknown-provider filtering, discovery failure behavior, empty-list "
        "rendering, URL-encoded /auth/login links, and known provider labels."
    ),
    argv=[
        "node",
        "--test",
        "crates/ironclaw_webui_v2_static/static/js/pages/login/login-oauth.test.mjs",
    ],
)

WEBUI_V2_LOGIN_BROWSER_MATRIX_COMMAND = CommandSpec(
    name="webui_v2_login_browser_matrix_contracts",
    description=(
        "Focused Playwright browser matrix for the committed Reborn WebUI v2 "
        "login/session bundle with stubbed public auth/session APIs: manual "
        "token trim and rejection, mobile layout, OAuth provider links, "
        "login-ticket exchange success/failure, sign-out local clear, stored "
        "token overwrite protection, fragment token precedence, and "
        "login_error callback banners."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "uv",
        "run",
        "--no-project",
        "--with",
        "pytest",
        "--with",
        "pytest-asyncio",
        "--with",
        "pytest-playwright",
        "--with",
        "pytest-timeout",
        "--with",
        "playwright",
        "--with",
        "aiohttp",
        "--with",
        "httpx",
        "--with",
        "cryptography",
        "pytest",
        "tests/e2e/scenarios/test_reborn_webui_v2_login_browser_matrix.py",
        "-q",
    ],
)

WEBUI_V2_CHAT_CLIENT_COMMAND = CommandSpec(
    name="webui_v2_chat_client_contracts",
    description=(
        "Focused WebUI v2 chat client node:test suite for send/retry state, "
        "pending-message reconciliation, approvals, auth gates, SSE timeline "
        "projection, history merge, markdown/readability, attachment staging, "
        "message grouping, cancellation, and thread-isolation contracts."
    ),
    argv=[
        "bash",
        "-lc",
        (
            "find crates/ironclaw_webui_v2_static/static/js/pages/chat "
            "-type f \\( -name '*test.mjs' -o -name '*test.js' \\) "
            "-print0 | xargs -0 node --test"
        ),
    ],
)

WEBUI_V2_CHAT_BROWSER_MATRIX_COMMAND = CommandSpec(
    name="webui_v2_chat_browser_matrix_contracts",
    description=(
        "Focused Playwright browser matrix for the committed Reborn WebUI v2 "
        "chat bundle with stubbed WebChat v2 APIs: first-message starter and "
        "typed sends, existing-thread follow-up, text/image attachments, "
        "picker/drop/paste validation, busy and failure recovery, retry, "
        "cancellation, keyboard, accessibility, focus, and mobile overflow."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "uv",
        "run",
        "--no-project",
        "--with",
        "pytest",
        "--with",
        "pytest-asyncio",
        "--with",
        "pytest-playwright",
        "--with",
        "pytest-timeout",
        "--with",
        "playwright",
        "--with",
        "aiohttp",
        "--with",
        "httpx",
        "--with",
        "cryptography",
        "pytest",
        "tests/e2e/scenarios/test_reborn_webui_v2_chat_browser_matrix.py",
        "-q",
    ],
)

WEBUI_V2_WORKSPACE_PROJECT_CLIENT_COMMAND = CommandSpec(
    name="webui_v2_workspace_project_client_contracts",
    description=(
        "Focused WebUI v2 workspace/projects client contracts for read-only "
        "filesystem mount browsing, safe file preview/download decisions, "
        "project overview/detail mapping, project mutation and membership "
        "route encoding, and still-stubbed mission/thread/widget helpers."
    ),
    argv=[
        "node",
        "--test",
        "crates/ironclaw_webui_v2_static/static/js/pages/workspace/lib/workspace-api.test.mjs",
        "crates/ironclaw_webui_v2_static/static/js/pages/projects/lib/projects-api.test.mjs",
    ],
)

WEBUI_V2_WORKSPACE_BROWSER_COMMAND = CommandSpec(
    name="webui_v2_workspace_browser_smoke",
    description=(
        "Playwright smoke for the WebUI v2 workspace file preview route "
        "through the real ironclaw-reborn serve binary with v2 filesystem "
        "mount/stat/content API responses."
    ),
    argv=[
        "uv",
        "run",
        "--no-project",
        "--with",
        "pytest",
        "--with",
        "pytest-asyncio",
        "--with",
        "pytest-playwright",
        "--with",
        "pytest-timeout",
        "--with",
        "playwright",
        "--with",
        "aiohttp",
        "--with",
        "httpx",
        "--with",
        "cryptography",
        "pytest",
        "tests/e2e/scenarios/test_reborn_webui_v2_smoke.py::test_reborn_v2_workspace_text_file_preview_uses_v2_fs_api",
        "-q",
    ],
)

WEBUI_V2_PROJECTS_BROWSER_COMMAND = CommandSpec(
    name="webui_v2_projects_browser_smoke",
    description=(
        "Playwright smoke for the WebUI v2 projects route through the real "
        "ironclaw-reborn serve binary: token stripping, authorized project "
        "overview fetch, search filtering, project-detail navigation, and "
        "authorized detail fetch."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "uv",
        "run",
        "--no-project",
        "--with",
        "pytest",
        "--with",
        "pytest-asyncio",
        "--with",
        "pytest-playwright",
        "--with",
        "pytest-timeout",
        "--with",
        "playwright",
        "--with",
        "aiohttp",
        "--with",
        "httpx",
        "--with",
        "cryptography",
        "pytest",
        "tests/e2e/scenarios/test_reborn_webui_v2_smoke.py::test_reborn_v2_projects_overview_filter_and_detail_browser_smoke",
        "-q",
    ],
)

WEBUI_V2_AUTOMATIONS_RUNTIME_TOOL_SUBSTRATE_COMMAND = CommandSpec(
    name="webui_v2_automations_runtime_tool_substrate_contracts",
    description=(
        "Runtime/tool substrate contracts below WebUI v2 automation and "
        "outbound workflows: authorization boundaries, runtime policy "
        "reductions, network egress policy, process lifecycle, sandbox "
        "guards, and loop-support event/checkpoint contracts."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_authorization",
        "-p",
        "ironclaw_runtime_policy",
        "-p",
        "ironclaw_network",
        "-p",
        "ironclaw_processes",
        "-p",
        "ironclaw_process_sandbox",
        "-p",
        "ironclaw_loop_support",
        "--all-features",
        "--jobs",
        "2",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_PROJECTS_CLIENT_API_COMMAND = CommandSpec(
    name="webui_v2_projects_client_api_contracts",
    description=(
        "Focused WebUI v2 projects client API contracts for project overview "
        "and detail mapping, project create/update/delete route selection, "
        "project membership route encoding, missing-id fail-closed behavior, "
        "and explicit mission/thread/widget TODO stubs."
    ),
    argv=[
        "node",
        "--test",
        "crates/ironclaw_webui_v2_static/static/js/pages/projects/lib/projects-api.test.mjs",
    ],
)

WEBUI_V2_AUTOMATIONS_CLIENT_COMMAND = CommandSpec(
    name="webui_v2_automations_client_contracts",
    description=(
        "Focused WebUI v2 automations/outbound-defaults client contracts for "
        "automation list/mutation routes, completed-row query toggles, "
        "schedule/summary/recent-run presenters, empty-state affordances, "
        "refresh cadence decisions, and outbound preference/target API payloads."
    ),
    argv=[
        "bash",
        "-lc",
        (
            "node --test crates/ironclaw_webui_v2_static/static/js/lib/api.test.mjs "
            "$(find crates/ironclaw_webui_v2_static/static/js/pages/automations "
            "-type f -name '*test.mjs' | sort)"
        ),
    ],
)

WEBUI_V2_AUTOMATIONS_BROWSER_COMMAND = CommandSpec(
    name="webui_v2_automations_browser_smoke",
    description=(
        "Playwright smoke for the WebUI v2 automations route through the "
        "real ironclaw-reborn serve binary: automation list query shape, "
        "outbound delivery target rendering, Slack final-reply target save "
        "payload, bearer propagation, and current-default UI state."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "uv",
        "run",
        "--no-project",
        "--with",
        "pytest",
        "--with",
        "pytest-asyncio",
        "--with",
        "pytest-playwright",
        "--with",
        "pytest-timeout",
        "--with",
        "playwright",
        "--with",
        "aiohttp",
        "--with",
        "httpx",
        "--with",
        "cryptography",
        "pytest",
        "tests/e2e/scenarios/test_reborn_webui_v2_smoke.py::test_reborn_v2_automations_delivery_default_browser_smoke",
        "-q",
    ],
)

WEBUI_V2_EXTENSIONS_CLIENT_COMMAND = CommandSpec(
    name="webui_v2_extensions_client_contracts",
    description=(
        "Focused WebUI v2 extensions/channel-pairing client contracts for "
        "extension registry/list/lifecycle/setup/OAuth API routes, registry "
        "presentation, lifecycle actions, channel and MCP tabs, Slack setup "
        "and allowed-channel helpers, pairing redemption, and user-safe "
        "pairing error mapping."
    ),
    argv=[
        "bash",
        "-lc",
        (
            "node --test "
            "crates/ironclaw_webui_v2_static/static/js/components/slack-channel-picker.test.mjs "
            "crates/ironclaw_webui_v2_static/static/js/components/slack-setup-panel.test.mjs "
            "crates/ironclaw_webui_v2_static/static/js/lib/channel-connect.test.mjs "
            "crates/ironclaw_webui_v2_static/static/js/lib/slack-channels-api.test.mjs "
            "crates/ironclaw_webui_v2_static/static/js/lib/slack-pairing-api.test.mjs "
            "crates/ironclaw_webui_v2_static/static/js/lib/slack-setup-api.test.mjs "
            "$(find crates/ironclaw_webui_v2_static/static/js/pages/extensions "
            "-type f -name '*test.mjs' | sort)"
        ),
    ],
)

WEBUI_V2_EXTENSIONS_BROWSER_COMMAND = CommandSpec(
    name="webui_v2_extensions_browser_smoke",
    description=(
        "Playwright smoke for the WebUI v2 extensions registry route through "
        "the real ironclaw-reborn serve binary: token stripping, registry "
        "card rendering, install payload, activate/remove empty bodies, "
        "bearer propagation, toast feedback, and lifecycle UI state."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "uv",
        "run",
        "--no-project",
        "--with",
        "pytest",
        "--with",
        "pytest-asyncio",
        "--with",
        "pytest-playwright",
        "--with",
        "pytest-timeout",
        "--with",
        "playwright",
        "--with",
        "aiohttp",
        "--with",
        "httpx",
        "--with",
        "cryptography",
        "pytest",
        "tests/e2e/scenarios/test_reborn_webui_v2_smoke.py::test_reborn_v2_extensions_lifecycle_browser_smoke",
        "-q",
    ],
)

WEBUI_V2_EXTENSION_LIFECYCLE_HANDLER_COMMAND = CommandSpec(
    name="webui_v2_extension_lifecycle_handler_contracts",
    description=(
        "Focused WebUI v2 extension lifecycle route contracts for list, "
        "registry, install, activate, remove, setup GET/POST, malformed "
        "package ids, caller scope, and facade dispatch."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_webui_v2",
        "--features",
        "webui-v2-beta",
        "--test",
        "webui_v2_handlers_contract",
        "extension_",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_EXTENSION_DESCRIPTOR_COMMAND = CommandSpec(
    name="webui_v2_extension_lifecycle_descriptor_contracts",
    description=(
        "WebUI v2 descriptor contract for extension lifecycle route ids, "
        "methods, patterns, body limits, rate limits, auth policies, audit "
        "classes, and ProductWorkflow effect-path classification."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_webui_v2",
        "--features",
        "webui-v2-beta",
        "--test",
        "webui_v2_descriptors_contract",
        "--",
        "--format",
        "terse",
    ],
)

COMPOSITION_EXTENSION_SETUP_ROUTE_COMMAND = CommandSpec(
    name="composition_webui_v2_extension_setup_route_contract",
    description=(
        "Composition-mounted WebUI v2 extension setup route contract proving "
        "the hosted router returns lifecycle setup projections through the "
        "facade without legacy status aliases."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "webui-v2-beta,test-support",
        "--test",
        "webui_v2_serve",
        "setup_extension_returns_lifecycle_projection_via_facade",
        "--",
        "--exact",
        "--format",
        "terse",
    ],
)

COMPOSITION_EXTENSION_LIFECYCLE_COMMAND = CommandSpec(
    name="composition_extension_lifecycle_service_contracts",
    description=(
        "Composition extension lifecycle service contracts for catalog "
        "install, activation, removal, setup projection, restoration, "
        "credentialed activation, store failures, and lifecycle events."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "webui-v2-beta,test-support",
        "extension_lifecycle",
        "--lib",
        "--",
        "--format",
        "terse",
    ],
)

WASM_PRODUCT_ADAPTER_RUNTIME_COMMAND = CommandSpec(
    name="wasm_product_adapter_runtime_contracts",
    description=(
        "WASM ProductAdapter runtime contracts used by extension/product "
        "adapter lifecycle paths for component-model adapter loading, host "
        "calls, auth evidence handling, egress restrictions, and component "
        "error mapping."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_wasm_product_adapters",
        "--jobs",
        "2",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_SKILL_MANAGEMENT_HANDLER_COMMAND = CommandSpec(
    name="webui_v2_skill_management_handler_contract",
    description=(
        "Focused WebUI v2 skill-management handler contract for list, "
        "search, install, read, update, remove, and per-skill "
        "auto-activation route dispatch through the actual axum router."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_webui_v2",
        "--features",
        "webui-v2-beta",
        "--test",
        "webui_v2_handlers_contract",
        "skill_routes_dispatch_to_facade_methods",
        "--",
        "--exact",
        "--format",
        "terse",
    ],
)

WEBUI_V2_SKILL_MANAGEMENT_DESCRIPTOR_COMMAND = CommandSpec(
    name="webui_v2_skill_management_descriptor_contract",
    description=(
        "WebUI v2 descriptor lock that includes the skill-management route "
        "methods, path patterns, auth policy, body limits, rate limits, "
        "audit classes, and allowed effect paths."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_webui_v2",
        "--features",
        "webui-v2-beta",
        "--test",
        "webui_v2_descriptors_contract",
        "every_descriptor_matches_the_locked_policy_surface",
        "--",
        "--exact",
        "--format",
        "terse",
    ],
)

COMPOSITION_SKILL_MANAGEMENT_COMMAND = CommandSpec(
    name="composition_skill_management_contracts",
    description=(
        "Composition skill-management contracts for local skill listing, "
        "bundled Reborn skill installation, skill lifecycle facade behavior, "
        "unsafe-content rejection, owner scoping, and local-dev capability "
        "writes to the user skill root."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "bash",
        "-lc",
        (
            "cargo test -p ironclaw_reborn_composition --features webui-v2-beta,test-support "
            "local_skill_list --lib -- --format terse "
            "&& cargo test -p ironclaw_reborn_composition --features webui-v2-beta,test-support "
            "bundled_reborn_skills --lib -- --format terse "
            "&& cargo test -p ironclaw_reborn_composition --features webui-v2-beta,test-support "
            "skill_lifecycle --lib -- --format terse "
            "&& cargo test -p ironclaw_reborn_composition --features webui-v2-beta,test-support "
            "skills_product_facade --lib -- --format terse "
            "&& cargo test -p ironclaw_reborn_composition --features webui-v2-beta,test-support "
            "local_dev_capability_port_skill_install_writes_user_skill_root --lib -- --format terse"
        ),
    ],
)

WEBUI_V2_SLACK_PAIRING_UI_COMMAND = CommandSpec(
    name="webui_v2_slack_pairing_ui_contracts",
    description=(
        "Focused WebUI v2 Slack proof-code pairing UI contracts for custom "
        "and localized copy, blank/pending submit disabling, trimmed button "
        "and Enter-key redemption, success/error messaging, query "
        "invalidation, route selection, and Slack-only inbound proof-code "
        "renderer gating."
    ),
    argv=[
        "node",
        "--test",
        "crates/ironclaw_webui_v2_static/static/js/components/slack-pairing-section.test.mjs",
        "crates/ironclaw_webui_v2_static/static/js/lib/slack-pairing-api.test.mjs",
        "crates/ironclaw_webui_v2_static/static/js/lib/channel-connect.test.mjs",
        "crates/ironclaw_webui_v2_static/static/js/pages/chat/components/channel-connect-card.test.mjs",
        "crates/ironclaw_webui_v2_static/static/js/pages/extensions/components/channels-tab.test.mjs",
    ],
)

WEBUI_V2_SLACK_PAIRING_BROWSER_COMMAND = CommandSpec(
    name="webui_v2_slack_pairing_browser_smoke",
    description=(
        "Playwright smoke for WebUI v2 Slack proof-code pairing through the "
        "real ironclaw-reborn serve binary: built-in Slack connect action "
        "rendering, trimmed button submit, Enter-key submit, success/error "
        "messages, bearer-authenticated v2 redeem POSTs, and no legacy v1 "
        "pairing browser calls."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "uv",
        "run",
        "--no-project",
        "--with",
        "pytest",
        "--with",
        "pytest-asyncio",
        "--with",
        "pytest-playwright",
        "--with",
        "pytest-timeout",
        "--with",
        "playwright",
        "--with",
        "aiohttp",
        "--with",
        "httpx",
        "--with",
        "cryptography",
        "pytest",
        "tests/e2e/scenarios/test_reborn_webui_v2_smoke.py::test_reborn_v2_slack_pairing_browser_success_error_and_keyboard_submit",
        "-q",
    ],
)

WEBUI_V2_SETTINGS_ONBOARDING_CLIENT_COMMAND = CommandSpec(
    name="webui_v2_settings_onboarding_client_contracts",
    description=(
        "Focused WebUI v2 settings/onboarding client contracts for provider "
        "classification and management, onboarding-gate routing, NEAR AI and "
        "Codex login helper safety, settings v2 API route selection, skills, "
        "traces, tools, and explicit users-tab stubs."
    ),
    argv=[
        "bash",
        "-lc",
        (
            "node --test "
            "crates/ironclaw_webui_v2_static/static/js/lib/onboarding-gate.test.js "
            "$(find crates/ironclaw_webui_v2_static/static/js/pages/settings "
            "-type f -name '*test.mjs' | sort)"
        ),
    ],
)

WEBUI_V2_ONBOARDING_PROVIDER_BROWSER_COMMAND = CommandSpec(
    name="webui_v2_onboarding_provider_browser_smoke",
    description=(
        "Playwright smoke for WebUI v2 first-run /welcome provider-login "
        "controls through the real ironclaw-reborn serve binary: NEAR AI "
        "Google hosted-login request origin/body, Codex device-login request "
        "and visible user code, and mobile setup-menu viewport containment."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "uv",
        "run",
        "--no-project",
        "--with",
        "pytest",
        "--with",
        "pytest-asyncio",
        "--with",
        "pytest-playwright",
        "--with",
        "pytest-timeout",
        "--with",
        "playwright",
        "--with",
        "aiohttp",
        "--with",
        "httpx",
        "--with",
        "cryptography",
        "pytest",
        "tests/e2e/scenarios/test_reborn_webui_v2_onboarding_provider_login_browser.py",
        "-q",
    ],
)

WEBUI_V2_I18N_LANGUAGE_COMMAND = CommandSpec(
    name="webui_v2_i18n_language_contracts",
    description=(
        "Focused WebUI v2 i18n and language-selection contracts for saved/"
        "navigator/default language detection, lazy locale-pack loading, "
        "concurrent import memoization, failed import retryability, stale-load "
        "protection, translation fallback, language search, current-language "
        "display, setLang routing, and empty-search rendering."
    ),
    argv=[
        "node",
        "--test",
        "crates/ironclaw_webui_v2_static/static/js/lib/i18n.test.mjs",
        "crates/ironclaw_webui_v2_static/static/js/pages/settings/components/language-tab.test.mjs",
    ],
)

WEBUI_V2_SETTINGS_SHELL_COMMAND = CommandSpec(
    name="webui_v2_settings_shell_role_gating_contracts",
    description=(
        "Focused WebUI v2 Settings shell contracts for admin/member default "
        "tabs, unknown-tab redirects, non-admin operator-tab redirects, "
        "desktop role filtering, mobile hidden-active fallback, and tab-change "
        "callbacks."
    ),
    argv=[
        "node",
        "--test",
        "crates/ironclaw_webui_v2_static/static/js/pages/settings/settings-shell.test.mjs",
    ],
)

WEBUI_V2_SETTINGS_RESTART_COMMAND = CommandSpec(
    name="webui_v2_settings_restart_banner_contracts",
    description=(
        "Focused WebUI v2 Settings restart banner contracts for needsRestart "
        "visibility, disabled v2 restart affordance, unavailable copy, local "
        "confirmation callbacks, and no legacy restart side effects."
    ),
    argv=[
        "node",
        "--test",
        "crates/ironclaw_webui_v2_static/static/js/pages/settings/settings-restart.test.mjs",
    ],
)

WEBUI_V2_SETTINGS_TOOLBAR_SEARCH_COMMAND = CommandSpec(
    name="webui_v2_settings_toolbar_search_contracts",
    description=(
        "Focused WebUI v2 Settings toolbar/search contracts for SettingsPage "
        "toolbar reachability, JSON import/export actions, search matching, "
        "settings-shell callback wiring, and v2 settings API route selection."
    ),
    argv=[
        "node",
        "--test",
        "crates/ironclaw_webui_v2_static/static/js/pages/settings/components/settings-toolbar.test.mjs",
        "crates/ironclaw_webui_v2_static/static/js/pages/settings/settings-shell.test.mjs",
        "crates/ironclaw_webui_v2_static/static/js/pages/settings/lib/settings-api.test.mjs",
    ],
)

WEBUI_V2_SETTINGS_DIRECT_TABS_COMMAND = CommandSpec(
    name="webui_v2_settings_direct_tabs_contracts",
    description=(
        "Focused WebUI v2 Settings direct-tab and configuration-panel "
        "contracts for direct route dispatch, role gating, schema-backed "
        "settings panels, restart affordance, channels, tools, and users."
    ),
    argv=[
        "node",
        "--test",
        "crates/ironclaw_webui_v2_static/static/js/pages/settings/settings-shell.test.mjs",
        "crates/ironclaw_webui_v2_static/static/js/pages/settings/settings-restart.test.mjs",
        "crates/ironclaw_webui_v2_static/static/js/pages/settings/components/settings-direct-tabs.test.mjs",
        "crates/ironclaw_webui_v2_static/static/js/pages/settings/components/tools-tab.test.mjs",
        "crates/ironclaw_webui_v2_static/static/js/pages/settings/lib/settings-api.test.mjs",
        "crates/ironclaw_webui_v2_static/static/js/pages/settings/lib/settings-schema.test.mjs",
    ],
)

WEBUI_V2_ADMIN_CLIENT_COMMAND = CommandSpec(
    name="webui_v2_admin_client_contracts",
    description=(
        "Focused WebUI v2 Admin console client contracts for page routing, "
        "tab navigation, fail-closed TODO API stubs, and admin usage/user "
        "presenter formatting, filtering, and aggregation."
    ),
    argv=[
        "node",
        "--test",
        "crates/ironclaw_webui_v2_static/static/js/pages/admin/admin-contracts.test.mjs",
    ],
)

WEBUI_V2_TOAST_QUERY_CLIENT_COMMAND = CommandSpec(
    name="webui_v2_toast_query_client_contracts",
    description=(
        "Focused WebUI v2 toast bus, ToastViewport, and shared QueryClient "
        "default contracts for notification delivery, cleanup, tone fallback, "
        "and bounded query-cache behavior."
    ),
    argv=[
        "node",
        "--test",
        "crates/ironclaw_webui_v2_static/static/js/lib/toast-query.test.mjs",
        "crates/ironclaw_webui_v2_static/static/js/layout/shell-static-contracts.test.mjs",
    ],
)

REBORN_CLI_TRIGGER_POLLER_SETTINGS_COMMAND = CommandSpec(
    name="reborn_cli_trigger_poller_settings_contracts",
    description=(
        "Focused Reborn CLI trigger-poller runtime settings contracts for "
        "run/serve defaults, config and environment overrides, strict parsing, "
        "interval validation, and runtime-input propagation used by WebUI v2 "
        "scheduled automations."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_cli",
        "trigger_poller",
        "--",
        "--format",
        "terse",
    ],
)

REBORN_CLI_CREDENTIAL_REFRESH_SETTINGS_COMMAND = CommandSpec(
    name="reborn_cli_credential_refresh_settings_contracts",
    description=(
        "Focused Reborn CLI credential-refresh runtime settings contracts "
        "for run/serve defaults, operator force-on and kill-switch env "
        "overrides, invalid env rejection, and RuntimeInput propagation."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_cli",
        "credential_refresh",
        "--",
        "--format",
        "terse",
    ],
)

REBORN_CLI_DOCKERFILE_COMMAND = CommandSpec(
    name="reborn_cli_dockerfile_contracts",
    description=(
        "Focused Reborn CLI Dockerfile contracts for WebUI v2, Slack host, "
        "libSQL/Postgres feature builds, shipped seed configs, migration "
        "copying, Railway-safe volume handling, and absent Docker VOLUME "
        "instructions."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_cli",
        "--test",
        "smoke",
        "dockerfile_reborn",
        "--",
        "--format",
        "terse",
    ],
)

REBORN_CLI_DOCKER_RAILWAY_ENTRYPOINT_COMMAND = CommandSpec(
    name="reborn_cli_docker_railway_entrypoint_contracts",
    description=(
        "Focused Reborn CLI Docker/Railway entrypoint contracts for local "
        "and production seed configs, Railway volume home selection, "
        "ephemeral local-dev rejection, stale-config rejection, and default "
        "config path safety."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_cli",
        "--test",
        "smoke",
        "docker_reborn",
        "--",
        "--format",
        "terse",
    ],
)

REBORN_CLI_WEBUI_V2_BINARY_COMMAND = CommandSpec(
    name="reborn_cli_webui_v2_binary",
    description=(
        "Build the Reborn CLI binary with the WebUI v2 serve surface before "
        "browser smokes launch target/debug/ironclaw-reborn."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "build",
        "-p",
        "ironclaw_reborn_cli",
        "--features",
        "webui-v2-beta",
        "--bin",
        "ironclaw-reborn",
    ],
)

WEBUI_V2_HIDDEN_STUBBED_ROUTE_COMMAND = CommandSpec(
    name="webui_v2_hidden_stubbed_route_contracts",
    description=(
        "Focused WebUI v2 hidden/stubbed direct-route contracts for hidden "
        "route metadata, registered direct routes, and jobs/routines/missions/"
        "admin TODO API adapters that must not call unsupported v1 endpoints, "
        "plus jobs/routines/missions shell presenter contracts."
    ),
    argv=[
        "node",
        "--test",
        "crates/ironclaw_webui_v2_static/static/js/app/routes.test.mjs",
        "crates/ironclaw_webui_v2_static/static/js/app/hidden-stub-apis.test.mjs",
        "crates/ironclaw_webui_v2_static/static/js/app/hidden-stub-presenters.test.mjs",
    ],
)

WEBUI_V2_HIDDEN_WORKFLOW_BROWSER_COMMAND = CommandSpec(
    name="webui_v2_hidden_workflow_direct_routes_browser_smoke",
    description=(
        "Focused Playwright smoke for Reborn WebUI v2 hidden Jobs, Missions, "
        "Routines, and Admin direct routes, verifying they render or redirect "
        "in Chromium without legacy v1-shaped browser API calls."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "uv",
        "run",
        "--no-project",
        "--with",
        "pytest",
        "--with",
        "pytest-asyncio",
        "--with",
        "pytest-playwright",
        "--with",
        "pytest-timeout",
        "--with",
        "playwright",
        "--with",
        "aiohttp",
        "--with",
        "httpx",
        "--with",
        "cryptography",
        "pytest",
        "tests/e2e/scenarios/test_reborn_webui_v2_smoke.py::test_reborn_v2_hidden_workflow_direct_routes_render_without_legacy_v1_calls",
        "tests/e2e/scenarios/test_reborn_webui_v2_smoke.py::test_reborn_v2_admin_hidden_route_redirects_by_capability",
        "-q",
    ],
)

WEBUI_V2_LOGS_CLIENT_COMMAND = CommandSpec(
    name="webui_v2_logs_client_contracts",
    description=(
        "Focused WebUI v2 logs screen client contracts for scoped filter "
        "normalization, polling/fallback behavior, unsupported operator route "
        "handling, empty/error states, page scroll layout, and chat/automation "
        "scoped log links."
    ),
    argv=[
        "node",
        "--test",
        "crates/ironclaw_webui_v2_static/static/js/pages/logs/lib/logs-data.test.mjs",
        "crates/ironclaw_webui_v2_static/static/js/pages/logs/hooks/useLogs.test.mjs",
        "crates/ironclaw_webui_v2_static/static/js/pages/logs/logs-page.test.mjs",
        "crates/ironclaw_webui_v2_static/static/js/pages/automations/components/automation-recent-runs.test.mjs",
        "crates/ironclaw_webui_v2_static/static/js/pages/chat/lib/chat.test.mjs",
    ],
)

WEBUI_V2_LOGS_BROWSER_COMMAND = CommandSpec(
    name="webui_v2_logs_browser_smoke",
    description=(
        "Focused Playwright smoke for the Reborn WebUI v2 logs route, "
        "verifying scoped URL filters are passed to /api/webchat/v2/operator/"
        "logs and rendered log context expands in Chromium."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "uv",
        "run",
        "--no-project",
        "--with",
        "pytest",
        "--with",
        "pytest-asyncio",
        "--with",
        "pytest-playwright",
        "--with",
        "pytest-timeout",
        "--with",
        "playwright",
        "--with",
        "aiohttp",
        "--with",
        "httpx",
        "--with",
        "cryptography",
        "pytest",
        "tests/e2e/scenarios/test_reborn_webui_v2_smoke.py::test_reborn_v2_logs_page_passes_scope_to_api_and_renders_context",
        "-q",
    ],
)

WEBUI_V2_SHELL_CLIENT_COMMAND = CommandSpec(
    name="webui_v2_shell_client_contracts",
    description=(
        "Focused WebUI v2 shell/navigation/session-control client contracts "
        "for onboarding redirects, sidebar responsiveness, route filtering, "
        "thread pin/search/delete behavior, command palette actions, account "
        "controls, toasts, and TEE/report affordances."
    ),
    argv=[
        "node",
        "--test",
        "crates/ironclaw_webui_v2_static/static/js/layout/shell-static-contracts.test.mjs",
        "crates/ironclaw_webui_v2_static/static/js/hooks/useSidebar.test.mjs",
        "crates/ironclaw_webui_v2_static/static/js/lib/onboarding-gate.test.js",
        "crates/ironclaw_webui_v2_static/static/js/lib/pin-store.test.js",
        "crates/ironclaw_webui_v2_static/static/js/lib/thread-errors.test.mjs",
        "crates/ironclaw_webui_v2_static/static/js/pages/chat/hooks/useThreads.test.mjs",
        "crates/ironclaw_webui_v2_static/static/js/app/routes.test.mjs",
    ],
)

WEBUI_V2_SHELL_BROWSER_COMMAND = CommandSpec(
    name="webui_v2_shell_browser_smoke",
    description=(
        "Playwright smoke for WebUI v2 shell navigation through the real "
        "ironclaw-reborn serve binary: command palette route jump, sidebar "
        "workspace navigation, and sidebar collapse/restore."
    ),
    argv=[
        "uv",
        "run",
        "--no-project",
        "--with",
        "pytest",
        "--with",
        "pytest-asyncio",
        "--with",
        "pytest-playwright",
        "--with",
        "pytest-timeout",
        "--with",
        "playwright",
        "--with",
        "aiohttp",
        "--with",
        "httpx",
        "--with",
        "cryptography",
        "pytest",
        "tests/e2e/scenarios/test_reborn_webui_v2_smoke.py::test_reborn_v2_shell_palette_and_sidebar_navigation",
        "-q",
    ],
)

WEBUI_V2_TEE_CLIENT_COMMAND = CommandSpec(
    name="webui_v2_tee_attestation_client_contracts",
    description=(
        "Focused WebUI v2 TEE attestation client contracts for public-host "
        "endpoint derivation, local/IP suppression, attestation/report fetch "
        "paths, clipboard payload formatting, hidden unavailable state, "
        "loading/error/copy UI states, and header integration."
    ),
    argv=[
        "node",
        "--test",
        "crates/ironclaw_webui_v2_static/static/js/lib/tee-attestation.test.mjs",
        "crates/ironclaw_webui_v2_static/static/js/layout/shell-static-contracts.test.mjs",
    ],
)

WEBUI_V2_TRACE_CREDITS_CLIENT_COMMAND = CommandSpec(
    name="webui_v2_trace_credits_client_contracts",
    description=(
        "Focused WebUI v2 sidebar Trace Commons credits client contracts for "
        "hidden loading/error/not-enrolled state, signed final-credit "
        "formatting, accepted/submitted count defaults, positive held-count "
        "copy, settings/traces navigation, shared trace-credits query key, "
        "and display-only sidebar wiring."
    ),
    argv=[
        "node",
        "--test",
        "crates/ironclaw_webui_v2_static/static/js/lib/trace-credits-card.test.mjs",
    ],
)

WEBUI_V2_TRACE_CREDITS_BROWSER_COMMAND = CommandSpec(
    name="webui_v2_trace_credits_browser_smoke",
    description=(
        "Served WebUI v2 browser smoke for the Trace Commons sidebar card: "
        "enrolled credit summary rendering, accepted/submitted and held-count "
        "copy, bearer-backed credit fetch, settings/traces navigation, and "
        "not-enrolled hidden state."
    ),
    argv=[
        "uv",
        "run",
        "--no-project",
        "--with",
        "pytest",
        "--with",
        "pytest-asyncio",
        "--with",
        "pytest-playwright",
        "--with",
        "pytest-timeout",
        "--with",
        "playwright",
        "--with",
        "aiohttp",
        "--with",
        "httpx",
        "--with",
        "cryptography",
        "pytest",
        "tests/e2e/scenarios/test_reborn_webui_v2_trace_credits_browser.py",
        "-q",
    ],
)

WEBUI_V2_OPERATOR_LOGS_HANDLER_COMMAND = CommandSpec(
    name="webui_v2_operator_logs_handler_contract",
    description=(
        "Focused WebUI v2 operator logs handler contract for enforcing "
        "operator capability before serving scoped operator log queries."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_webui_v2",
        "--features",
        "webui-v2-beta",
        "--test",
        "webui_v2_handlers_contract",
        "operator_logs_require_operator_capability",
        "--",
        "--exact",
        "--format",
        "terse",
    ],
)

REBORN_OPERATOR_LOGS_SERVICE_COMMAND = CommandSpec(
    name="reborn_operator_logs_service_contracts",
    description=(
        "Focused Reborn operator log buffer contracts for bounded in-memory "
        "retention, newest-first and cursor queries, level/target/correlation "
        "filters, tracing-layer capture, secret/path redaction, UTF-8 "
        "truncation, response byte caps, tail, and follow cursors."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--lib",
        "operator_logs",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_OPERATOR_LOGS_ROUTE_DISPATCH_COMMAND = CommandSpec(
    name="webui_v2_operator_logs_route_dispatch_contract",
    description=(
        "Focused WebUI v2 operator log route contract for dispatching "
        "bounded query parameters when operator capability is present."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_webui_v2",
        "--features",
        "webui-v2-beta",
        "--test",
        "webui_v2_handlers_contract",
        "operator_routes_dispatch_to_facade_with_body_and_query_inputs",
        "--",
        "--exact",
        "--format",
        "terse",
    ],
)

WEBUI_V2_OPERATOR_LOGS_BROWSER_COMMAND = CommandSpec(
    name="webui_v2_operator_logs_browser_smoke",
    description=(
        "Served WebUI v2 browser smoke for the operator logs page: URL "
        "scope query propagation to the API, scoped entry rendering, context "
        "expansion, and correlation chips."
    ),
    argv=[
        "uv",
        "run",
        "--no-project",
        "--with",
        "pytest",
        "--with",
        "pytest-asyncio",
        "--with",
        "pytest-playwright",
        "--with",
        "pytest-timeout",
        "--with",
        "playwright",
        "--with",
        "aiohttp",
        "--with",
        "httpx",
        "--with",
        "cryptography",
        "pytest",
        "tests/e2e/scenarios/test_reborn_webui_v2_smoke.py::test_reborn_v2_logs_page_passes_scope_to_api_and_renders_context",
        "-q",
    ],
)

SLACK_PERSONAL_BINDING_ROUTE_COMMAND = CommandSpec(
    name="slack_personal_binding_oauth_route_contracts",
    description=(
        "Focused Slack personal-binding OAuth route contracts for "
        "bearer-protected start, sanitized redirect handling, single-use "
        "callback state, denied/missing-code/provider-failure paths, binding "
        "mismatch, store failure, expiry, and per-user pending-state eviction."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "slack-v2-host-beta",
        "slack_personal_binding_serve",
        "--lib",
        "--",
        "--format",
        "terse",
    ],
)

SLACK_PERSONAL_BINDING_SERVICE_COMMAND = CommandSpec(
    name="slack_personal_binding_service_contracts",
    description=(
        "Focused Slack personal-binding service contracts for tenant/app/team/"
        "installation validation, app-scoped installation enforcement, invalid "
        "Slack id rejection, and binding-store error propagation."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "slack-v2-host-beta",
        "slack_personal_binding::tests",
        "--lib",
        "--",
        "--format",
        "terse",
    ],
)

SLACK_PERSONAL_PAIRING_ROUTE_COMMAND = CommandSpec(
    name="slack_personal_pairing_redeem_route_contracts",
    description=(
        "Focused Slack personal pairing WebUI route contracts for bearer-bound "
        "code redemption, invalid/unknown/foreign-tenant code handling, "
        "unsupported-channel rejection, and binding-store failure mapping."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "slack-v2-host-beta",
        "slack_personal_binding_pairing_serve",
        "--lib",
        "--",
        "--format",
        "terse",
    ],
)

SLACK_PERSONAL_PAIRING_SERVICE_COMMAND = CommandSpec(
    name="slack_personal_pairing_service_contracts",
    description=(
        "Focused Slack personal pairing service contracts for code validation, "
        "tenant-scoped challenge consumption, foreign/unknown code rejection, "
        "challenge issue failures, resolver challenge issuance, cooldown, "
        "non-Slack shapes, and lookup/issue error propagation."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "slack-v2-host-beta",
        "slack_personal_binding_pairing::tests",
        "--lib",
        "--",
        "--format",
        "terse",
    ],
)

SLACK_HOST_BETA_WEBUI_ONLY_CLI_COMMAND = CommandSpec(
    name="slack_host_beta_webui_only_cli_contracts",
    description=(
        "Focused CLI contracts proving Slack host-beta enablement fails closed "
        "when the binary is built with WebUI v2 but without the Slack host-beta "
        "feature."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_cli",
        "--features",
        "webui-v2-beta",
        "serve_slack",
        "--bin",
        "ironclaw-reborn",
        "--",
        "--format",
        "terse",
    ],
)

SLACK_HOST_BETA_CLI_SERVE_COMMAND = CommandSpec(
    name="slack_host_beta_cli_serve_mount_smoke",
    description=(
        "Caller-level ironclaw-reborn serve smoke proving env-enabled Slack "
        "mounts the Slack Events API route instead of returning 404."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_cli",
        "--features",
        "webui-v2-beta,slack-v2-host-beta",
        "--test",
        "smoke",
        "serve_env_slack_enabled_mounts_slack_events_route",
        "--",
        "--exact",
        "--format",
        "terse",
    ],
)

SLACK_HOST_BETA_COMPOSITION_COMMAND = CommandSpec(
    name="slack_host_beta_composition_contracts",
    description=(
        "Focused composition contracts for building signed Slack Events API "
        "mounts, pairing redeem routes, channel routing, dispatch, and "
        "fail-closed runtime dependencies without live Slack traffic."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "slack-v2-host-beta",
        "slack_host_beta",
        "--lib",
        "--",
        "--format",
        "terse",
    ],
)

SLACK_EVENTS_INGRESS_COMMAND = CommandSpec(
    name="slack_events_ingress_contracts",
    description=(
        "Focused Slack Events ingress contracts for URL verification, signed "
        "event dispatch, malformed/unknown/ambiguous installation rejection, "
        "capacity/rate-limit mapping, adapter panic/timeout mapping, route "
        "descriptor policy, and e2e ProductAdapter flow behavior."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "slack-v2-host-beta",
        "slack_serve",
        "--lib",
        "--",
        "--format",
        "terse",
    ],
)

SLACK_SHARED_CHANNEL_ADMIN_COMMAND = CommandSpec(
    name="slack_shared_channel_admin_contracts",
    description=(
        "Focused Slack shared-channel admin and target contracts for route "
        "list/upsert/delete, operator-only access, subject validation, "
        "stored/static route merging, outbound target listing, owner changes, "
        "target authority revocation, route visibility gating, duplicate "
        "channel-route rejection, and binding-ref validation."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "slack-v2-host-beta",
        "slack_channel",
        "--lib",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_SLACK_CHANNEL_ADMIN_CLIENT_COMMAND = CommandSpec(
    name="webui_v2_slack_channel_admin_client_contracts",
    description=(
        "Focused WebUI v2 Slack shared-channel admin client contracts for "
        "allowed-channel normalization, list/save route selection, explicit "
        "subject payloads, partial subject preservation, setup-panel dirty "
        "field protection, secret validation, and picker error states."
    ),
    argv=[
        "node",
        "--test",
        "crates/ironclaw_webui_v2_static/static/js/components/slack-channel-picker.test.mjs",
        "crates/ironclaw_webui_v2_static/static/js/components/slack-setup-panel.test.mjs",
        "crates/ironclaw_webui_v2_static/static/js/lib/slack-channels-api.test.mjs",
    ],
)

SLACK_DELIVERY_COMMAND = CommandSpec(
    name="slack_delivery_contracts",
    description=(
        "Focused Slack outbound delivery contracts for accepted/deferred/rejected "
        "run acknowledgements, final reply delivery, approval/auth prompt "
        "rendering, timeout/error status recording, duplicate suppression, "
        "delivery permits, and personal-DM enforcement."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "slack-v2-host-beta",
        "slack_delivery",
        "--lib",
        "--",
        "--format",
        "terse",
    ],
)

SLACK_EGRESS_COMMAND = CommandSpec(
    name="slack_egress_contracts",
    description=(
        "Focused Slack host-mediated egress contracts for HTTPS host policy, "
        "opaque credential-handle bearer injection, control-character rejection, "
        "runtime HTTP failure mapping, and fresh invocation scope per send."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "slack-v2-host-beta",
        "slack_egress",
        "--lib",
        "--",
        "--format",
        "terse",
    ],
)

SLACK_OUTBOUND_TARGETS_COMMAND = CommandSpec(
    name="slack_outbound_targets_contracts",
    description=(
        "Focused Slack outbound target contracts for shared-channel and personal "
        "DM target listing, binding-ref parsing, tenant/user isolation, target "
        "caps, and Slack id validation."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "slack-v2-host-beta",
        "slack_outbound_targets",
        "--lib",
        "--",
        "--format",
        "terse",
    ],
)

SLACK_DM_OPEN_COMMAND = CommandSpec(
    name="slack_dm_open_contracts",
    description=(
        "Focused Slack personal-DM open contracts for successful channel id "
        "extraction, non-2xx/oversized/missing-channel failures, and DM channel "
        "id shape validation."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "slack-v2-host-beta",
        "slack_dm_open",
        "--lib",
        "--",
        "--format",
        "terse",
    ],
)

SLACK_ADAPTER_COMMAND = CommandSpec(
    name="slack_v2_adapter_render_delivery_contracts",
    description=(
        "Focused Slack v2 adapter contracts for final-reply rendering, long "
        "message chunking, Slack mrkdwn conversion, auth prompts, status "
        "recording, partial multipart retry suppression, and token-safe "
        "ok:false handling."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_slack_v2_adapter",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_SEND_MULTILINE_COMMAND = CommandSpec(
    name="webui_v2_send_multiline_contract",
    description="Focused send-message route contract for preserving multiline content.",
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_webui_v2",
        "--features",
        "webui-v2-beta",
        "send_message_preserves_multiline_content",
        "--",
        "--exact",
    ],
)

WEBUI_V2_SEND_ERROR_COMMAND = CommandSpec(
    name="webui_v2_send_error_contract",
    description="Focused send-message route contract for sanitized service errors.",
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_webui_v2",
        "--features",
        "webui-v2-beta",
        "send_message_service_error_maps_to_sanitized_http_response",
        "--",
        "--exact",
    ],
)

WEBUI_V2_CANCEL_ERROR_COMMAND = CommandSpec(
    name="webui_v2_cancel_error_contract",
    description="Focused cancel-run route contract for sanitized service errors.",
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_webui_v2",
        "--features",
        "webui-v2-beta",
        "cancel_run_service_error_maps_to_sanitized_http_response",
        "--",
        "--exact",
    ],
)

WEBUI_V2_SESSION_THREAD_MESSAGE_HANDLER_COMMAND = CommandSpec(
    name="webui_v2_session_thread_message_handler_contract",
    description=(
        "Focused WebUI v2 session/thread/message route-family contract for "
        "session identity, thread create/list/delete, message submission, "
        "timeline pagination, and attachment download plumbing."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_webui_v2",
        "--features",
        "webui-v2-beta",
        "--test",
        "webui_v2_handlers_contract",
        "session_thread_message_routes_dispatch_to_facade_methods",
        "--",
        "--exact",
        "--format",
        "terse",
    ],
)

WEBUI_V2_STREAMING_RUN_CONTROL_HANDLER_COMMAND = CommandSpec(
    name="webui_v2_streaming_run_control_handler_contract",
    description=(
        "Focused WebUI v2 streaming/run-control route-family contract for "
        "SSE event subscriptions, cursor precedence, run cancellation, and "
        "approval/auth gate resolution plumbing."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_webui_v2",
        "--features",
        "webui-v2-beta",
        "--test",
        "webui_v2_handlers_contract",
        "streaming_run_control_routes_dispatch_to_facade_methods",
        "--",
        "--exact",
        "--format",
        "terse",
    ],
)

WEBUI_V2_AUTOMATIONS_TRACE_OUTBOUND_CHANNEL_HANDLER_COMMAND = CommandSpec(
    name="webui_v2_automations_trace_outbound_channel_handler_contract",
    description=(
        "Focused WebUI v2 automations/trace/outbound/channel route-family "
        "contract for automation list and mutations, trace credit and hold "
        "authorization routes, outbound preferences/targets, connectable "
        "channels, and malformed automation query rejection."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_webui_v2",
        "--features",
        "webui-v2-beta",
        "--test",
        "webui_v2_handlers_contract",
        "automations_trace_outbound_channel_routes_dispatch_to_facade_methods",
        "--",
        "--exact",
        "--format",
        "terse",
    ],
)

WEBUI_V2_FS_HANDLER_COMMAND = CommandSpec(
    name="webui_v2_filesystem_handler_slice",
    description="Focused WebUI v2 filesystem handler negative-path contract slice.",
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_webui_v2",
        "fs_",
        "--test",
        "webui_v2_handlers_contract",
    ],
)

WEBUI_V2_PROJECT_HANDLER_COMMAND = CommandSpec(
    name="webui_v2_project_handler_contracts",
    description=(
        "Focused WebUI v2 project route contracts for list/unwired handling, "
        "project path/body precedence, member add routing, and delete responses."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_webui_v2",
        "--features",
        "webui-v2-beta",
        "--test",
        "webui_v2_handlers_contract",
        "project_",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_PROJECTS_HANDLER_COMMAND = CommandSpec(
    name="webui_v2_projects_handler_contracts",
    description=(
        "Focused WebUI v2 projects collection/session contracts for unwired "
        "project service handling and reborn-projects feature projection."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_webui_v2",
        "--features",
        "webui-v2-beta",
        "--test",
        "webui_v2_handlers_contract",
        "projects",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_MEMBER_HANDLER_COMMAND = CommandSpec(
    name="webui_v2_member_handler_contracts",
    description=(
        "Focused WebUI v2 project-member route contracts for add/update/remove "
        "path/body precedence and no-content delete responses."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_webui_v2",
        "--features",
        "webui-v2-beta",
        "--test",
        "webui_v2_handlers_contract",
        "member",
        "--",
        "--format",
        "terse",
    ],
)

COMPOSITION_PROJECT_SERVICE_COMMAND = CommandSpec(
    name="composition_project_service_contracts",
    description=(
        "Focused Reborn project service contracts for project ACL enforcement, "
        "revoked-access filtering, revoked-member mutation rejection, and "
        "owner role projection on project creation."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--lib",
        "project_service",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_SSO_AUTH_ROUTE_COMMAND = CommandSpec(
    name="webui_v2_sso_auth_route_contracts",
    description=(
        "WebUI v2 auth route contracts for bearer/session/OIDC acceptance, "
        "rejection, route mounting, and generic unauthorized responses."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_webui_ingress",
        "--all-features",
        "--test",
        "auth_route_contract",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_GOOGLE_OAUTH_ROUTE_COMMAND = CommandSpec(
    name="webui_v2_google_oauth_routes",
    description=(
        "Google SSO public route contracts for provider discovery, login "
        "redirect, callback success/failure, state replay, ticket exchange, "
        "logout, open-redirect defense, and hosted-domain denial."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_webui_ingress",
        "--all-features",
        "--test",
        "google_oauth_routes",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_GITHUB_OAUTH_ROUTE_COMMAND = CommandSpec(
    name="webui_v2_github_oauth_routes",
    description=(
        "GitHub SSO public route contracts for provider discovery, login "
        "redirect, callback success/failure, state replay, verified-email "
        "selection, ticket exchange, and logout."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_webui_ingress",
        "--all-features",
        "--test",
        "github_oauth_routes",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_SESSION_ROUND_TRIP_COMMAND = CommandSpec(
    name="webui_v2_sso_session_round_trip",
    description=(
        "End-to-end WebUI v2 SSO callback, one-time ticket exchange, "
        "protected route bearer use, logout, and revoked-session rejection."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_webui_ingress",
        "--all-features",
        "--test",
        "session_round_trip",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_INGRESS_SESSION_AUTH_COMMAND = CommandSpec(
    name="webui_v2_ingress_session_auth_contracts",
    description=(
        "Focused ingress auth/session contracts for exact env bearer matching, "
        "empty or wrong token rejection, session creation/lookup/expiry, "
        "single-use tickets, revoked-session denial, tenant isolation, signed "
        "session round-trips, and protected-route authentication."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_webui_ingress",
        "--all-features",
        "session",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_SSO_NETWORK_LIMITS_COMMAND = CommandSpec(
    name="webui_v2_sso_network_limits",
    description=(
        "SSO public route rate-limit, body-limit, and CORS fail-closed "
        "contracts for login, session exchange, and logout."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_webui_ingress",
        "--all-features",
        "--test",
        "network_limits_contract",
        "sso_",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_SSO_PUBLIC_MOUNT_COMMAND = CommandSpec(
    name="webui_v2_sso_public_mount_policy",
    description=(
        "Composition-level public route mount contract proving /auth/providers "
        "is reachable without bearer auth while protected WebUI v2 routes "
        "remain bearer-protected."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "webui-v2-beta,test-support",
        "--test",
        "webui_v2_serve",
        "public_route_mount_is_merged_without_bearer_auth_and_keeps_descriptor_policy",
        "--",
        "--exact",
        "--format",
        "terse",
    ],
)

WEBUI_V2_PUBLIC_SSO_OWNER_CRATE_COMMAND = CommandSpec(
    name="webui_v2_public_sso_owner_crate_contracts",
    description=(
        "Full WebUI ingress owner-crate regression for public SSO/session: "
        "auth routes, Google/GitHub provider routes, OIDC, signed sessions, "
        "headers/errors, network limits, serve loop, and session round trips."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_webui_ingress",
        "--all-features",
        "--",
        "--format",
        "terse",
    ],
)

REBORN_IDENTITY_FOUNDATION_COMMAND = CommandSpec(
    name="reborn_identity_foundation_contracts",
    description=(
        "Reborn identity foundation contracts for stable provider identity to "
        "user mapping, tenant scoping, verified email linking, concurrent "
        "first-login convergence, migration adoption, and key validation."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_identity",
        "--",
        "--format",
        "terse",
    ],
)

COMPOSITION_PROJECT_FS_COMMAND = CommandSpec(
    name="composition_project_filesystem_reader",
    description=(
        "Composition project filesystem reader scoping, path, hidden-file, "
        "oversize, MIME, and not-found contracts."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "webui-v2-beta,test-support",
        "project_filesystem_reader",
        "--lib",
        "--",
        "--format",
        "terse",
    ],
)

COMPOSITION_MOUNT_FS_COMMAND = CommandSpec(
    name="composition_mount_filesystem_reader",
    description="Composition mount filesystem reader traversal and policy contracts.",
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "mount_filesystem_reader",
        "--lib",
    ],
)

WEBUI_V2_HANDLER_CONTRACT_COMMAND = CommandSpec(
    name="webui_v2_handler_contract_file",
    description="Full WebUI v2 handler contract test file.",
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_webui_v2",
        "--test",
        "webui_v2_handlers_contract",
    ],
)

WEBUI_V2_WALLET_CONNECT_BROWSER_COMMAND = CommandSpec(
    name="webui_v2_wallet_connect_browser_smoke",
    description=(
        "Playwright smoke for the served WebUI v2 NEAR wallet connect popup: "
        "intercepts the remote wallet connector module with a deterministic "
        "browser stub, verifies the fixed NEAR AI sign-message request, and "
        "observes the BroadcastChannel success payload without live wallet "
        "traffic."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "uv",
        "run",
        "--no-project",
        "--with",
        "pytest",
        "--with",
        "pytest-asyncio",
        "--with",
        "pytest-playwright",
        "--with",
        "pytest-timeout",
        "--with",
        "playwright",
        "--with",
        "aiohttp",
        "--with",
        "httpx",
        "--with",
        "cryptography",
        "pytest",
        "tests/e2e/scenarios/test_reborn_webui_v2_wallet_connect_browser.py",
        "-q",
    ],
)

WEBUI_V2_RUST_STATIC_COMMAND = CommandSpec(
    name="webui_v2_rust_static_regression",
    description=(
        "Native WebUI v2 Rust route package plus embedded static asset/router "
        "package under all features."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_webui_v2",
        "-p",
        "ironclaw_webui_v2_static",
        "--all-features",
        "--jobs",
        "2",
    ],
)

WEBUI_V2_COMPOSITION_COMMAND = CommandSpec(
    name="webui_v2_composition_regression",
    description=(
        "Composed Reborn WebUI v2 gateway regression covering serve, runtime e2e, "
        "product-auth, middleware, static assets, SSE, and WebSocket policy."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "webui-v2-beta,test-support",
        "--test",
        "webui_v2_serve",
        "--test",
        "webui_v2_e2e",
        "--test",
        "webui_v2_product_auth",
        "--test",
        "webui_v2_product_auth_4201",
    ],
)

WEBUI_V2_COMPOSITION_STATIC_COMMAND = CommandSpec(
    name="webui_v2_composition_static_route_contracts",
    description=(
        "Composition-level static route contracts for /v2 root no-bearer "
        "access, direct client route fallback, JS/CSS content types, fresh "
        "CSP nonce substitution, static security headers, and unknown "
        "extension asset 404s."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "webui-v2-beta,test-support",
        "--test",
        "webui_v2_serve",
        "static",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_PRODUCT_AUTH_OAUTH_COMMAND = CommandSpec(
    name="webui_v2_product_auth_oauth_routes",
    description=(
        "Generic product-auth OAuth start/callback route contracts for flow "
        "creation, callback completion, bearer auth, sanitized invalid input, "
        "body limits, and per-caller rate limits."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "webui-v2-beta,test-support",
        "--test",
        "webui_v2_product_auth",
        "product_auth_oauth",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_PRODUCT_AUTH_GOOGLE_OAUTH_COMMAND = CommandSpec(
    name="webui_v2_product_auth_google_oauth_routes",
    description=(
        "Google product-auth OAuth route contracts for authorization URL "
        "construction, missing config, scope/expiry validation, callback "
        "completion, provider denial, unknown state, and secret-free browser "
        "completion notification."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "webui-v2-beta,test-support",
        "--test",
        "webui_v2_product_auth",
        "product_auth_google_oauth",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_PRODUCT_AUTH_CALLBACK_COMMAND = CommandSpec(
    name="webui_v2_product_auth_callback_routes",
    description=(
        "Product-auth OAuth callback contracts for malformed fields and flow "
        "ids, unknown flows, provider denial/exchange failures, cross-scope "
        "rejection, no-body enforcement, per-IP rate limits, and spoofed "
        "forwarded-header resistance."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "webui-v2-beta,test-support",
        "--test",
        "webui_v2_product_auth",
        "product_auth_callback",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_PRODUCT_AUTH_SERVICE_SUBSTRATE_COMMAND = CommandSpec(
    name="webui_v2_product_auth_service_substrate_contracts",
    description=(
        "Product-auth service substrate contracts behind WebUI v2 OAuth, "
        "manual-token, and account routes: auth/OAuth flow state, provider "
        "exchange/refresh boundaries, product workflow auth gates, adapters, "
        "outbound target resolution, triggers, projects, and conversations."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_auth",
        "-p",
        "ironclaw_oauth",
        "-p",
        "ironclaw_product_workflow",
        "-p",
        "ironclaw_product_adapters",
        "-p",
        "ironclaw_outbound",
        "-p",
        "ironclaw_triggers",
        "-p",
        "ironclaw_projects",
        "-p",
        "ironclaw_conversations",
        "--all-features",
        "--jobs",
        "2",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_EXTENSION_OAUTH_ROUTE_COMMAND = CommandSpec(
    name="webui_v2_extension_oauth_route_contract",
    description=(
        "WebUI v2 extension OAuth setup route contract for package-scoped "
        "update binding on the browser-facing setup endpoint."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "webui-v2-beta,test-support",
        "--test",
        "webui_v2_product_auth",
        "extension_oauth",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_EXTENSION_OAUTH_START_COMMAND = CommandSpec(
    name="webui_v2_extension_oauth_start_contracts",
    description=(
        "Extension OAuth start service contracts for DCR setup, reconnect "
        "binding to an existing owner account, cross-owner rejection, and "
        "missing DCR registry fail-closed behavior."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "webui-v2-beta,test-support",
        "extension_oauth_start",
        "--lib",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_EXTENSION_GOOGLE_OAUTH_COMMAND = CommandSpec(
    name="webui_v2_extension_google_oauth_start_contracts",
    description=(
        "Google extension OAuth start service contracts for existing-account "
        "binding, cross-thread rebind, and unavailable binding lookup fallback."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "webui-v2-beta,test-support",
        "extension_google_oauth_start",
        "--lib",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_DCR_OAUTH_CALLBACK_COMMAND = CommandSpec(
    name="webui_v2_dcr_oauth_callback_contracts",
    description=(
        "DCR OAuth callback contracts for callback state decoding, PKCE "
        "registry fallback, and blocked-turn gate resume."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "webui-v2-beta,test-support",
        "dcr_oauth_callback",
        "--lib",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_MANUAL_TOKEN_LEGACY_COMMAND = CommandSpec(
    name="webui_v2_manual_token_legacy_submit_routes",
    description=(
        "Legacy product-auth manual-token submit route contracts for bearer "
        "auth, redacted credential refs, invalid-secret handling, abandoned "
        "interactions on submit failure, setup errors, body limits, "
        "per-caller rate limits, and sanitized invalid fields."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "webui-v2-beta,test-support",
        "--test",
        "webui_v2_product_auth",
        "product_auth_manual_token",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_MANUAL_TOKEN_SPLIT_COMMAND = CommandSpec(
    name="webui_v2_manual_token_split_routes",
    description=(
        "Split manual-token setup/secret-submit route contracts for redacted "
        "projection, partial continuation rejection, invalid interaction "
        "sanitization, invocation-id enforcement, empty provider/label "
        "validation, and seeded gate challenge projection."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "webui-v2-beta,test-support",
        "--test",
        "webui_v2_product_auth_4201",
        "manual_token",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_MANUAL_TOKEN_FACADE_COMMAND = CommandSpec(
    name="webui_v2_manual_token_facade_contracts",
    description=(
        "Product-auth manual-token facade contracts for secret redaction, "
        "auth-flow tracking, completed-flow retry after continuation "
        "failure, cross-scope denial, stale/duplicate/malformed submit "
        "fail-closed behavior, sanitized backend failures, and cleanup on "
        "flow creation/completion failure."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "webui-v2-beta,test-support",
        "--test",
        "manual_tokens",
        "manual_token_facade",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_MANUAL_TOKEN_POSTGRES_MIGRATION_FACADE_COMMAND = CommandSpec(
    name="webui_v2_manual_token_postgres_migration_facade_contract",
    description=(
        "Postgres-feature facade_factory contract proving migration-dry-run "
        "validates the planned-turn process-port profile as a normal, "
        "non-ignored test."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "postgres",
        "--test",
        "facade_factory",
        "migration_dry_run_validates_postgres_planned_turn_profile",
        "--",
        "--exact",
        "--format",
        "terse",
    ],
)

WEBUI_V2_MANUAL_TOKEN_POSTGRES_FACADE_COMMAND = CommandSpec(
    name="webui_v2_manual_token_postgres_facade_contracts",
    description=(
        "Postgres-feature facade_factory contracts proving Postgres-only "
        "local-dev product-auth manual-token setup stays usable and does not "
        "advertise an unavailable durable backend."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "postgres",
        "--test",
        "facade_factory",
        "--jobs",
        "2",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_MANUAL_TOKEN_LIBSQL_FACADE_COMMAND = CommandSpec(
    name="webui_v2_manual_token_libsql_facade_contracts",
    description=(
        "libSQL-feature facade_factory contracts proving durable local-dev "
        "product-auth manual-token setup, persistence, and recovery remain "
        "usable through the WebUI-facing facade."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "libsql",
        "--test",
        "facade_factory",
        "--jobs",
        "2",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_ACCOUNT_ROUTE_COMMAND = CommandSpec(
    name="webui_v2_product_auth_account_routes",
    description=(
        "Product-auth account route contracts for listing configured accounts, "
        "selecting redacted projections, recovery/setup status, credential "
        "refresh, malformed account ids, wrong-provider or foreign-scope "
        "accounts, unknown accounts, missing invocation ids, and the tighter "
        "refresh rate limit."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "webui-v2-beta,test-support",
        "--test",
        "webui_v2_product_auth_4201",
        "account",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_LIFECYCLE_CLEANUP_COMMAND = CommandSpec(
    name="webui_v2_product_auth_lifecycle_cleanup_routes",
    description=(
        "Product-auth lifecycle cleanup route contracts for redacted cleanup "
        "reports, service dispatch, invalid extension id rejection, and "
        "secret-free responses."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "webui-v2-beta,test-support",
        "--test",
        "webui_v2_product_auth_4201",
        "lifecycle",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_DESCRIPTOR_POLICY_COMMAND = CommandSpec(
    name="webui_v2_descriptor_policy_surface",
    description=(
        "Locked WebUI v2 descriptor policy surface, including LLM provider and "
        "operator configuration route auth, body-limit, rate-limit, audit, and "
        "effect-path contracts."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_webui_v2",
        "--features",
        "webui-v2-beta",
        "--test",
        "webui_v2_descriptors_contract",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_OPERATOR_HANDLER_COMMAND = CommandSpec(
    name="webui_v2_operator_handler_contracts",
    description=(
        "Focused WebUI v2 operator setup, config, diagnostics, status, logs, "
        "service lifecycle, and capability-enforcement handler contracts."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_webui_v2",
        "--features",
        "webui-v2-beta",
        "--test",
        "webui_v2_handlers_contract",
        "operator_",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_OPERATOR_MOUNT_COMMAND = CommandSpec(
    name="webui_v2_operator_mount_policy",
    description=(
        "Composition-level session capability and operator-only route mount "
        "policy for WebUI v2 LLM/operator configuration APIs."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "webui-v2-beta,test-support",
        "--test",
        "webui_v2_serve",
        "operator",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_OPERATOR_LLM_CONFIG_COMMAND = CommandSpec(
    name="webui_v2_operator_llm_config_persistence",
    description=(
        "Composed operator LLM-config smoke covering NEAR AI provider key "
        "persistence, active provider selection, and read-back after re-save."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "webui-v2-beta,test-support,root-llm-provider",
        "--test",
        "webui_v2_e2e",
        "operator_llm_config",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_LLM_PROVIDER_ROUTE_COMMAND = CommandSpec(
    name="webui_v2_llm_provider_routes",
    description=(
        "Focused WebUI v2 LLM provider route contracts for provider CRUD, "
        "test/list-model dispatch, NEAR AI login/wallet routes, Codex login, "
        "and operator-capability enforcement."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_webui_v2",
        "--features",
        "webui-v2-beta",
        "--test",
        "webui_v2_handlers_contract",
        "llm_provider_routes",
        "--",
        "--format",
        "terse",
    ],
)

IRONCLAW_LLM_PROVIDER_SUBSTRATE_COMMAND = CommandSpec(
    name="ironclaw_llm_provider_substrate_contracts",
    description=(
        "Focused LLM owner-crate substrate contracts for provider request and "
        "response conversion, auth parsing, model classification, costs, "
        "retry, failover, circuit-breaker, embedding provider, URL safety, "
        "and embedding-cache behavior used by WebUI v2 operator/provider "
        "configuration and memory/search flows."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_llm",
        "-p",
        "ironclaw_embeddings",
        "--jobs",
        "2",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_NEARAI_LOGIN_STATE_COMMAND = CommandSpec(
    name="webui_v2_nearai_login_state_contracts",
    description=(
        "NEAR AI login one-time state, origin sanitization, and public "
        "callback descriptor policy contracts."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "webui-v2-beta,test-support,root-llm-provider",
        "nearai_login",
        "--lib",
        "--",
        "--format",
        "terse",
    ],
)

WEBUI_V2_PROVIDER_LOGIN_MOUNT_COMMAND = CommandSpec(
    name="webui_v2_provider_login_multi_user_mount_policy",
    description=(
        "Composition-level policy that operator-only LLM provider and "
        "provider-login routes are not mounted for multi-user authenticators."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "cargo",
        "test",
        "-p",
        "ironclaw_reborn_composition",
        "--features",
        "webui-v2-beta,test-support",
        "--test",
        "webui_v2_serve",
        "operator_routes_are_not_mounted_for_multi_user_authenticator",
        "--",
        "--exact",
        "--format",
        "terse",
    ],
)

WEBUI_V2_PROVIDER_LOGIN_BROWSER_COMMAND = CommandSpec(
    name="webui_v2_provider_login_browser_smoke",
    description=(
        "Playwright smoke for WebUI v2 Settings provider-login controls "
        "through the real ironclaw-reborn serve binary: NEAR AI hosted-login "
        "request origin/body, Codex device-login request and visible user "
        "code, and browser-visible NEAR AI/Codex start-failure errors."
    ),
    env={"CARGO_INCREMENTAL": "0"},
    argv=[
        "uv",
        "run",
        "--no-project",
        "--with",
        "pytest",
        "--with",
        "pytest-asyncio",
        "--with",
        "pytest-playwright",
        "--with",
        "pytest-timeout",
        "--with",
        "playwright",
        "--with",
        "aiohttp",
        "--with",
        "httpx",
        "--with",
        "cryptography",
        "pytest",
        "tests/e2e/scenarios/test_reborn_webui_v2_provider_login_browser.py",
        "-q",
    ],
)

CASES: dict[str, CaseSpec] = {
    "webui_v2_serve_listener_regression": CaseSpec(
        name="webui_v2_serve_listener_regression",
        feature="WebUI v2 serve listener",
        category="Hermetic WebUI v2 CLI Serve Listener Regression",
        qa_matrix_test_ids=[
            "REBCLI-033-TC-01",
            "REBCLI-033-TC-02",
            "REBCLI-033-TC-03",
            "REBCLI-033-TC-04",
            "REBCLI-033-TC-05",
            "REBCLI-033-TC-06",
            "REBCLI-033-TC-07",
        ],
        commands=[WEBUI_V2_SERVE_LISTENER_CLI_COMMAND],
        notes=(
            "Covers the CLI-owned WebUI serve listener rows without browser "
            "duplication: help surface, missing token/user fail-closed "
            "startup, config seeding before binding, malformed host rejection, "
            "ephemeral test port startup, and trusted-laptop host-access "
            "listener guardrails."
        ),
    ),
    "webui_v2_serve_security_config_regression": CaseSpec(
        name="webui_v2_serve_security_config_regression",
        feature="WebUI v2 serve security configuration",
        category="Hermetic WebUI v2 CLI Serve Security Regression",
        qa_matrix_test_ids=[
            "REBCLI-034-TC-01",
            "REBCLI-034-TC-02",
            "REBCLI-034-TC-03",
            "REBCLI-034-TC-04",
            "REBCLI-034-TC-05",
            "REBCLI-034-TC-06",
        ],
        commands=[
            WEBUI_V2_SERVE_SECURITY_CLI_COMMAND,
            WEBUI_V2_SERVE_CORS_COMMAND,
            WEBUI_V2_SERVE_BODY_LIMIT_COMMAND,
            WEBUI_V2_SERVE_WS_ORIGIN_COMMAND,
            WEBUI_V2_DESCRIPTOR_POLICY_COMMAND,
        ],
        notes=(
            "Covers the CLI-owned WebUI serve security-configuration rows "
            "without browser duplication: invalid canonical host, invalid "
            "allowed origin, zero body fallback, CORS allow/deny behavior, "
            "descriptor body caps, WebSocket same-origin policy, and "
            "canonical-host override behavior."
        ),
    ),
    "webui_v2_sso_login_startup_regression": CaseSpec(
        name="webui_v2_sso_login_startup_regression",
        feature="WebUI v2 SSO login startup",
        category="Hermetic WebUI v2 SSO Startup Regression",
        qa_matrix_test_ids=[
            "REBCLI-035-TC-01",
            "REBCLI-035-TC-02",
            "REBCLI-035-TC-03",
            "REBCLI-035-TC-04",
            "REBCLI-035-TC-05",
            "REBCLI-035-TC-06",
            "REBCLI-035-TC-07",
        ],
        commands=[
            WEBUI_V2_SSO_STARTUP_SMOKE_COMMAND,
            WEBUI_V2_SSO_STARTUP_HELPER_COMMAND,
        ],
        notes=(
            "Covers the CLI-owned WebUI SSO startup rows without live OAuth "
            "provider calls: no-provider None behavior, provider without "
            "admission allowlist fail-closed before binding, missing "
            "provider secret failures, allowed-domain normalization, explicit "
            "base URL precedence, listener fallback URL, loopback cleartext "
            "allowance, and public cleartext rejection."
        ),
    ),
    "webui_v2_sso_user_admission_regression": CaseSpec(
        name="webui_v2_sso_user_admission_regression",
        feature="WebUI v2 SSO user admission",
        category="Hermetic WebUI v2 SSO User Admission Regression",
        qa_matrix_test_ids=[
            "REBCLI-036-TC-01",
            "REBCLI-036-TC-02",
            "REBCLI-036-TC-03",
            "REBCLI-036-TC-04",
            "REBCLI-036-TC-05",
            "REBCLI-036-TC-06",
            "REBCLI-036-TC-07",
        ],
        commands=[WEBUI_V2_SSO_USER_ADMISSION_COMMAND],
        notes=(
            "Covers the CLI-owned WebUI SSO user-admission adapter without "
            "live OAuth provider calls: verified allowlisted canonical email, "
            "off-list rejection, unverified and missing-email rejection, "
            "case-insensitive domains, allowlisted verified secondary email, "
            "tenant separation, and local trigger-access seed/no-seed behavior."
        ),
    ),
    "webui_v2_auth_surface_composition_regression": CaseSpec(
        name="webui_v2_auth_surface_composition_regression",
        feature="WebUI v2 auth surface composition",
        category="Hermetic WebUI v2 Auth Surface Composition Regression",
        qa_matrix_test_ids=[
            "REBCLI-037-TC-01",
            "REBCLI-037-TC-02",
            "REBCLI-037-TC-03",
            "REBCLI-037-TC-04",
            "REBCLI-037-TC-05",
            "REBCLI-037-TC-06",
            "REBCLI-037-TC-07",
        ],
        commands=[WEBUI_V2_AUTH_SURFACE_COMMAND],
        notes=(
            "Covers the CLI-owned WebUI auth-surface composition rows without "
            "live OAuth provider calls: env-bearer-only serve mounts no public "
            "login routes, configured SSO fails closed without an identity "
            "resolver, and configured SSO builds the signed-session/public "
            "login surface with local trigger-access bootstrap wiring."
        ),
    ),
    "openai_compat_beta_routes_regression": CaseSpec(
        name="openai_compat_beta_routes_regression",
        feature="OpenAI-Compatible Beta Routes",
        category="Hermetic WebUI v2/OpenAI-Compatible Route Mount Regression",
        qa_matrix_test_ids=[
            "REBCLI-039-TC-01",
            "REBCLI-039-TC-02",
            "REBCLI-039-TC-03",
            "REBCLI-039-TC-04",
            "REBCLI-039-TC-05",
            "REBCLI-039-TC-06",
            "REBCLI-039-TC-07",
            "REBCLI-039-TC-08",
        ],
        commands=[
            OPENAI_COMPAT_ROUTE_MOUNT_COMMAND,
            OPENAI_COMPAT_ALL_FEATURE_COMPOSITION_COMMAND,
        ],
        notes=(
            "Hermetic coverage for the Reborn serve/composition boundary that "
            "PR #5348's browser canary work does not own: OpenAI-compatible "
            "protected route mounts, WebUI bearer-auth gating, ProductWorkflow "
            "submission/readback, feature-gated all-feature composition, and "
            "shared turn-admission behavior without live OpenAI traffic."
        ),
    ),
    "openai_compat_owner_crate_regression": CaseSpec(
        name="openai_compat_owner_crate_regression",
        feature="OpenAI-compatible Chat Completions and Responses API",
        category="Hermetic Owner-Crate Regression",
        qa_matrix_test_ids=["REBCLI-056-TC-07"],
        commands=[OPENAI_OWNER_CRATE_COMMAND],
        notes=(
            "Matches the QA matrix owner-crate command for REBCLI-056-TC-07; "
            "the same cargo command also exercises Responses API owner-crate "
            "behavior, but only the explicit spreadsheet row is counted here."
        ),
    ),
    "openai_responses_api_workflow_regression": CaseSpec(
        name="openai_responses_api_workflow_regression",
        feature="OpenAI-compatible Responses create, retrieve, and cancel APIs",
        category="Hermetic Responses API Handler Contract",
        qa_matrix_test_ids=[
            "REBCLI-057-TC-01",
            "REBCLI-057-TC-02",
            "REBCLI-057-TC-03",
            "REBCLI-057-TC-04",
            "REBCLI-057-TC-05",
            "REBCLI-057-TC-06",
            "REBCLI-058-TC-01",
            "REBCLI-058-TC-02",
            "REBCLI-058-TC-03",
            "REBCLI-058-TC-04",
            "REBCLI-058-TC-05",
            "REBCLI-058-TC-06",
        ],
        commands=[OPENAI_RESPONSES_WORKFLOW_COMMAND],
        notes=(
            "Focused ResponsesAPI contract coverage that PR #5348 does not "
            "duplicate: create on /api/v1 and /v1, retrieve/cancel, auth, "
            "invalid input, unsupported fields, wait timeout, cross-scope "
            "not-found shape, and sanitized ProductWorkflow errors."
        ),
    ),
    "openai_chat_completions_workflow_regression": CaseSpec(
        name="openai_chat_completions_workflow_regression",
        feature="OpenAI-compatible Chat Completions API",
        category="Hermetic Chat Completions Handler Contract",
        qa_matrix_test_ids=[
            "REBCLI-056-TC-01",
            "REBCLI-056-TC-02",
            "REBCLI-056-TC-03",
            "REBCLI-056-TC-04",
            "REBCLI-056-TC-05",
            "REBCLI-056-TC-06",
        ],
        commands=[OPENAI_CHAT_WORKFLOW_COMMAND],
        notes=(
            "Focused Chat Completions contract coverage that PR #5348 does "
            "not duplicate: non-stream success, idempotency replay/conflict, "
            "malformed JSON, model/idempotency validation, streaming "
            "guardrails, projection metadata, and sanitized ProductWorkflow "
            "errors."
        ),
    ),
    "support_substrate_product_workflow_regression": CaseSpec(
        name="support_substrate_product_workflow_regression",
        feature="WebUI v2 support substrates and product workflow idempotency",
        category="Hermetic Support Substrate Regression",
        qa_matrix_test_ids=[
            "REBCLI-043-TC-12",
            "REBCLI-044-TC-07",
            "REBCLI-045-TC-10",
            "REBCLI-047-TC-07",
            "REBCLI-056-TC-08",
        ],
        commands=[
            PRODUCT_WORKFLOW_LEDGER_COMMAND,
            SUPPORT_SUBSTRATE_COMMAND,
        ],
        notes=(
            "Runs the focused durable ledger contract first, then the broad "
            "iteration-182 support-substrate command referenced by the QA matrix."
        ),
    ),
    "webui_v2_session_thread_message_api_regression": CaseSpec(
        name="webui_v2_session_thread_message_api_regression",
        feature="WebUI v2 session, thread, and message APIs",
        category="Hermetic WebUI v2 API Regression",
        qa_matrix_test_ids=[
            "REBCLI-043-TC-01",
            "REBCLI-043-TC-02",
            "REBCLI-043-TC-03",
            "REBCLI-043-TC-04",
            "REBCLI-043-TC-05",
            "REBCLI-043-TC-06",
            "REBCLI-043-TC-09",
            "REBCLI-043-TC-10",
            "REBCLI-043-TC-11",
        ],
        commands=[
            WEBUI_V2_SESSION_THREAD_MESSAGE_HANDLER_COMMAND,
            WEBUI_V2_SESSION_SERVICE_SUBSTRATE_COMMAND,
            WEBUI_V2_SESSION_EXECUTION_SUBSTRATE_COMMAND,
        ],
        notes=(
            "Runs a focused caller-level WebUI v2 router contract for the "
            "session/thread/message API family, then the service and "
            "execution substrate sweeps that preserve caller scope, turn "
            "state, persistence, gates, capabilities, and host runtime "
            "contracts after route admission. This is hermetic Rust coverage "
            "and intentionally does not duplicate PR #5348 browser Playwright "
            "ports."
        ),
    ),
    "webui_v2_streaming_run_control_api_regression": CaseSpec(
        name="webui_v2_streaming_run_control_api_regression",
        feature="WebUI v2 streaming and run-control APIs",
        category="Hermetic WebUI v2 API Regression",
        qa_matrix_test_ids=[
            "REBCLI-044-TC-01",
            "REBCLI-044-TC-02",
            "REBCLI-044-TC-03",
            "REBCLI-044-TC-04",
            "REBCLI-044-TC-05",
            "REBCLI-044-TC-06",
        ],
        commands=[WEBUI_V2_STREAMING_RUN_CONTROL_HANDLER_COMMAND],
        notes=(
            "Runs a focused caller-level WebUI v2 router contract for SSE "
            "event subscriptions, cursor handling, cancel, and gate "
            "resolution. Browser approval UX overlap remains referenced to "
            "PR #5348 instead of duplicated in this matrix branch."
        ),
    ),
    "webui_v2_automations_trace_outbound_channel_api_regression": CaseSpec(
        name="webui_v2_automations_trace_outbound_channel_api_regression",
        feature="WebUI v2 automations, trace, outbound, and channel APIs",
        category="Hermetic WebUI v2 API Regression",
        qa_matrix_test_ids=[
            "REBCLI-045-TC-01",
            "REBCLI-045-TC-02",
            "REBCLI-045-TC-03",
            "REBCLI-045-TC-04",
            "REBCLI-045-TC-05",
            "REBCLI-045-TC-06",
            "REBCLI-045-TC-07",
            "REBCLI-045-TC-08",
            "REBCLI-045-TC-09",
        ],
        commands=[
            WEBUI_V2_AUTOMATIONS_TRACE_OUTBOUND_CHANNEL_HANDLER_COMMAND,
            WEBUI_V2_SESSION_SERVICE_SUBSTRATE_COMMAND,
            WEBUI_V2_SESSION_EXECUTION_SUBSTRATE_COMMAND,
            WEBUI_V2_AUTOMATIONS_RUNTIME_TOOL_SUBSTRATE_COMMAND,
        ],
        notes=(
            "Runs a focused caller-level WebUI v2 router contract for "
            "automations, Trace Commons credit/hold authorization, outbound "
            "preferences and targets, connectable channels, and malformed "
            "automation query rejection, then service, execution, and "
            "runtime/tool substrate sweeps for outbound delivery, trigger "
            "execution, run notification state, capability gates, network "
            "egress, process lifecycle, sandboxing, and loop-support "
            "contracts. Static automations screen coverage remains mapped "
            "separately to REBCLI-067, and PR #5348 browser duplicates stay "
            "referenced instead of reimplemented here."
        ),
    ),
    "webui_v2_route_contract_regression": CaseSpec(
        name="webui_v2_route_contract_regression",
        feature="WebUI v2 chat route contracts",
        category="Hermetic Route Contract",
        qa_matrix_test_ids=[
            "REBCLI-055-TC-08",
            "REBCLI-065-TC-23",
            "REBCLI-065-TC-24",
            "REBCLI-065-TC-25",
        ],
        commands=[
            WEBUI_V2_SEND_MULTILINE_COMMAND,
            WEBUI_V2_SEND_ERROR_COMMAND,
            WEBUI_V2_CANCEL_ERROR_COMMAND,
            WEBUI_V2_ROUTE_CONTRACT_COMMAND,
        ],
        notes=(
            "Runs the three focused WebUI v2 chat route contracts from the QA "
            "matrix, then the full native ironclaw_webui_v2 package check."
        ),
    ),
    "webui_v2_gateway_middleware_serve_foundation_regression": CaseSpec(
        name="webui_v2_gateway_middleware_serve_foundation_regression",
        feature="WebUI v2 gateway middleware and serve contract",
        category="Hermetic Gateway Middleware/Serve Foundation Regression",
        qa_matrix_test_ids=[
            "REBCLI-055-TC-01",
            "REBCLI-055-TC-02",
            "REBCLI-055-TC-03",
            "REBCLI-055-TC-04",
            "REBCLI-055-TC-05",
            "REBCLI-055-TC-06",
            "REBCLI-055-TC-10",
            "REBCLI-055-TC-11",
            "REBCLI-055-TC-14",
            "REBCLI-055-TC-15",
            "REBCLI-055-TC-16",
            "REBCLI-055-TC-17",
        ],
        commands=[
            WEBUI_V2_SERVE_LISTENER_CLI_COMMAND,
            WEBUI_V2_SERVE_SECURITY_CLI_COMMAND,
            WEBUI_V2_SERVE_CORS_COMMAND,
            WEBUI_V2_SERVE_BODY_LIMIT_COMMAND,
            WEBUI_V2_SERVE_WS_ORIGIN_COMMAND,
            WEBUI_V2_DESCRIPTOR_POLICY_COMMAND,
            WEBUI_V2_COMPOSITION_STATIC_COMMAND,
            WEBUI_V2_COMPOSITION_COMMAND,
            REBORN_COMPOSITION_ALL_FEATURE_COMMAND,
            REBORN_EVENT_STORE_FOUNDATION_COMMAND,
            WEBUI_V2_SESSION_EXECUTION_SUBSTRATE_COMMAND,
            REBORN_RUNTIME_TOOL_SUBSTRATE_COMMAND,
            REBORN_HOOK_BACKEND_ARCHITECTURE_COMMAND,
            REBORN_HOOK_POSTGRES_FEATURE_COMMAND,
            REBORN_HOOK_POSTGRES_PARITY_INTEGRATION_COMMAND,
        ],
        notes=(
            "Maps the remaining hermetic REBCLI-055 foundation rows into the "
            "canonical runner without duplicating PR #5348 browser/live "
            "coverage. TC-18/TC-19 remain live side-effect canaries in the "
            "separate live QA lane because they require external Google "
            "credentials."
        ),
    ),
    "webui_v2_static_js_regression": CaseSpec(
        name="webui_v2_static_js_regression",
        feature="WebUI v2 static browser-facing SPA modules",
        category="WebUI Static JavaScript Regression",
        qa_matrix_test_ids=[
            "REBCLI-055-TC-07",
            "REBCLI-055-TC-12",
        ],
        commands=[WEBUI_V2_STATIC_JS_COMMAND],
        notes=(
            "Runs the full discovered static/js node:test suite for the "
            "committed WebUI v2 SPA modules. This complements Rust route and "
            "composition checks without duplicating PR #5348's legacy "
            "Playwright browser port."
        ),
    ),
    "webui_v2_client_persistence_static_discovery_regression": CaseSpec(
        name="webui_v2_client_persistence_static_discovery_regression",
        feature="WebUI v2 client persistence and static JS test discovery",
        category="Hermetic Client Persistence/Static Discovery Regression",
        qa_matrix_test_ids=[
            "REBCLI-092-TC-01",
            "REBCLI-092-TC-02",
            "REBCLI-092-TC-03",
            "REBCLI-092-TC-04",
            "REBCLI-092-TC-05",
            "REBCLI-092-TC-06",
        ],
        commands=[WEBUI_V2_CLIENT_PERSISTENCE_DISCOVERY_COMMAND],
        notes=(
            "Covers WebUI v2 client persistence/static-discovery rows "
            "without duplicating PR #5348 browser legacy coverage: frontend "
            "npm test discovers both .test.js and .test.mjs static suites, "
            "including API error helpers, onboarding-gate decisions, "
            "pin-store and draft-store auth scoping/fallbacks, project-file "
            "path extraction/formatting, message grouping, and tool activity "
            "state."
        ),
    ),
    "webui_v2_chat_client_regression": CaseSpec(
        name="webui_v2_chat_client_regression",
        feature="WebUI v2 chat screen and gate UX",
        category="Hermetic Chat Client Contract Regression",
        qa_matrix_test_ids=[
            "REBCLI-065-TC-01",
            "REBCLI-065-TC-02",
            "REBCLI-065-TC-03",
            "REBCLI-065-TC-04",
            "REBCLI-065-TC-05",
            "REBCLI-065-TC-06",
            "REBCLI-065-TC-26",
            "REBCLI-065-TC-28",
        ],
        commands=[WEBUI_V2_CHAT_CLIENT_COMMAND],
        notes=(
            "Covers the six generated WebUI v2 chat/gate UX rows at the "
            "static client contract layer without re-porting PR #5348 browser "
            "legacy Playwright scenarios: send/retry state, pending-message "
            "reconciliation, approvals, auth gates, SSE timeline projection, "
            "history merge, markdown/readability, attachment staging, message "
            "grouping, cancellation, thread isolation, failed first-message "
            "retry metadata preservation, and explicit localized composer "
            "aria-label coverage."
        ),
    ),
    "webui_v2_chat_browser_matrix_regression": CaseSpec(
        name="webui_v2_chat_browser_matrix_regression",
        feature="WebUI v2 chat screen and gate UX",
        category="Hermetic Chat Browser Matrix Regression",
        qa_matrix_test_ids=[
            "REBCLI-065-TC-07",
            "REBCLI-065-TC-08",
            "REBCLI-065-TC-09",
            "REBCLI-065-TC-10",
            "REBCLI-065-TC-11",
            "REBCLI-065-TC-12",
            "REBCLI-065-TC-13",
            "REBCLI-065-TC-14",
            "REBCLI-065-TC-15",
            "REBCLI-065-TC-16",
            "REBCLI-065-TC-17",
            "REBCLI-065-TC-18",
            "REBCLI-065-TC-19",
            "REBCLI-065-TC-20",
            "REBCLI-065-TC-21",
            "REBCLI-065-TC-22",
            "REBCLI-065-TC-27",
            "REBCLI-065-TC-29",
            "REBCLI-065-TC-30",
            "REBCLI-065-TC-31",
            "REBCLI-065-TC-32",
            "REBCLI-065-TC-33",
            "REBCLI-065-TC-34",
            "REBCLI-065-TC-35",
            "REBCLI-065-TC-36",
        ],
        commands=[WEBUI_V2_CHAT_BROWSER_MATRIX_COMMAND],
        notes=(
            "Runs the committed WebUI v2 chat bundle in Chromium while "
            "stubbing only the WebChat v2 browser API and EventSource stream. "
            "Covers the real-browser matrix rows without live LLM calls: "
            "starter and typed first-message sends, existing-thread follow-up, "
            "attachment picker/drop/paste wire shapes and validation, image "
            "thumbnail rendering, busy/failure/retry behavior, cancellation, "
            "keyboard multiline submit, accessibility landmarks/named "
            "controls, focus restoration, and mobile no-overflow smoke."
        ),
    ),
    "webui_v2_workspace_project_client_regression": CaseSpec(
        name="webui_v2_workspace_project_client_regression",
        feature="WebUI v2 workspace and project browser screens",
        category="Hermetic Workspace/Project Client Contract Regression",
        qa_matrix_test_ids=[
            "REBCLI-066-TC-01",
            "REBCLI-066-TC-02",
            "REBCLI-066-TC-03",
            "REBCLI-066-TC-04",
            "REBCLI-066-TC-05",
            "REBCLI-066-TC-06",
            "REBCLI-066-TC-20",
            "REBCLI-084-TC-01",
            "REBCLI-084-TC-02",
            "REBCLI-084-TC-03",
            "REBCLI-084-TC-04",
            "REBCLI-084-TC-05",
            "REBCLI-084-TC-06",
            "REBCLI-084-TC-07",
        ],
        commands=[
            REBORN_CLI_WEBUI_V2_BINARY_COMMAND,
            WEBUI_V2_WORKSPACE_PROJECT_CLIENT_COMMAND,
            WEBUI_V2_PROJECTS_BROWSER_COMMAND,
            WEBUI_V2_WORKSPACE_BROWSER_COMMAND,
        ],
        notes=(
            "Covers the generated WebUI v2 workspace/project rows at the "
            "static client contract layer plus a Reborn WebUI v2 browser "
            "projects overview/detail smoke and workspace preview smoke: "
            "root mount browsing, mount-qualified "
            "directory entries, mount-root directory handling, bounded text "
            "and image preview via authed bytes, oversized text/image "
            "download-only behavior, unknown MIME UTF-8 versus binary "
            "sniffing, known-binary download-only behavior, Chromium text-file "
            "preview rendering through v2 fs mount/stat/content calls, "
            "browser project overview filtering/detail navigation through "
            "authorized v2 project calls, project overview/detail mapping, "
            "project create/update payloads, "
            "membership route encoding, and TODO subresource stubs that must "
            "not call unsupported v1 APIs."
        ),
    ),
    "webui_v2_automations_client_regression": CaseSpec(
        name="webui_v2_automations_client_regression",
        feature="WebUI v2 automations and outbound delivery defaults screen",
        category="Hermetic Automations Client Contract Regression",
        qa_matrix_test_ids=[
            "REBCLI-067-TC-01",
            "REBCLI-067-TC-02",
            "REBCLI-067-TC-03",
            "REBCLI-067-TC-04",
            "REBCLI-067-TC-05",
            "REBCLI-067-TC-06",
            "REBCLI-067-TC-07",
        ],
        commands=[
            REBORN_CLI_WEBUI_V2_BINARY_COMMAND,
            WEBUI_V2_AUTOMATIONS_CLIENT_COMMAND,
            WEBUI_V2_AUTOMATIONS_BROWSER_COMMAND,
        ],
        notes=(
            "Covers the generated WebUI v2 automations/outbound-default rows "
            "at the static client and browser layers without duplicating PR "
            "#5348 browser legacy Playwright scenarios: automation list/mutation "
            "routes, completed-row query toggles, schedule labels/timezones, "
            "filter/summary/recent-run presentation, empty-state copy/start "
            "actions, refresh cadence bounds, and outbound preference/target "
            "API payloads including clear-to-null, plus browser Slack "
            "final-reply target selection, save body, bearer propagation, and "
            "current-default UI state."
        ),
    ),
    "webui_v2_extensions_client_regression": CaseSpec(
        name="webui_v2_extensions_client_regression",
        feature="WebUI v2 extensions and channel pairing screens",
        category="Hermetic Extensions Client Contract Regression",
        qa_matrix_test_ids=[
            "REBCLI-068-TC-01",
            "REBCLI-068-TC-02",
            "REBCLI-068-TC-03",
            "REBCLI-068-TC-04",
            "REBCLI-068-TC-05",
            "REBCLI-068-TC-06",
            "REBCLI-068-TC-16",
        ],
        commands=[
            REBORN_CLI_WEBUI_V2_BINARY_COMMAND,
            WEBUI_V2_EXTENSIONS_CLIENT_COMMAND,
            WEBUI_V2_EXTENSIONS_BROWSER_COMMAND,
        ],
        notes=(
            "Covers the generated WebUI v2 extensions/channel-pairing rows "
            "at the static client and browser layers without duplicating PR "
            "#5348 browser legacy Playwright scenarios: extension registry/list/"
            "install/activate/remove/setup/OAuth API routes, registry and card "
            "presentation, lifecycle action selection/toasts, configure modal "
            "behavior, channel and MCP tab wiring, Slack setup and allowed "
            "channel helpers, proof-code pairing redemption, and user-safe "
            "pairing error mapping, plus browser lifecycle install, activate, "
            "remove, token stripping, bearer propagation, and mutation payload "
            "checks."
        ),
    ),
    "webui_v2_extension_lifecycle_api_regression": CaseSpec(
        name="webui_v2_extension_lifecycle_api_regression",
        feature="WebUI v2 extension lifecycle APIs",
        category="Hermetic Extension Lifecycle API Regression",
        qa_matrix_test_ids=[
            "REBCLI-046-TC-01",
            "REBCLI-046-TC-02",
            "REBCLI-046-TC-03",
            "REBCLI-046-TC-04",
            "REBCLI-046-TC-05",
            "REBCLI-046-TC-06",
            "REBCLI-046-TC-08",
        ],
        commands=[
            WEBUI_V2_EXTENSION_LIFECYCLE_HANDLER_COMMAND,
            WEBUI_V2_EXTENSION_DESCRIPTOR_COMMAND,
            COMPOSITION_EXTENSION_SETUP_ROUTE_COMMAND,
            COMPOSITION_EXTENSION_LIFECYCLE_COMMAND,
            WASM_PRODUCT_ADAPTER_RUNTIME_COMMAND,
        ],
        notes=(
            "Covers the generated WebUI v2 extension lifecycle API rows "
            "without duplicating PR #5348 browser legacy scenarios: list/"
            "registry reads, install/activate/remove mutations, setup GET and "
            "POST, malformed package-id rejection, caller-scoped facade "
            "dispatch, descriptor route policy/body-limit/effect-path "
            "lockstep, composition-mounted setup projection behavior, "
            "lifecycle service install/activation/removal/restoration "
            "contracts, and WASM ProductAdapter runtime dependency contracts."
        ),
    ),
    "webui_v2_skill_management_api_regression": CaseSpec(
        name="webui_v2_skill_management_api_regression",
        feature="WebUI v2 skill management APIs",
        category="Hermetic Skill Management API Regression",
        qa_matrix_test_ids=[
            "REBCLI-047-TC-01",
            "REBCLI-047-TC-02",
            "REBCLI-047-TC-03",
            "REBCLI-047-TC-04",
            "REBCLI-047-TC-05",
            "REBCLI-047-TC-06",
        ],
        commands=[
            WEBUI_V2_SKILL_MANAGEMENT_HANDLER_COMMAND,
            WEBUI_V2_SKILL_MANAGEMENT_DESCRIPTOR_COMMAND,
            COMPOSITION_SKILL_MANAGEMENT_COMMAND,
        ],
        notes=(
            "Covers the generated WebUI v2 skill-management API rows without "
            "duplicating PR #5348 browser legacy scenarios: actual axum "
            "handler dispatch for list/search/install/read/update/remove and "
            "per-skill auto-activation; descriptor method/path/auth/body/"
            "rate/effect-path policy lockstep; and composition skill listing, "
            "bundled skill installation, lifecycle facade, scoped owner "
            "visibility, unsafe-content rejection, and local-dev capability "
            "skill-root write contracts."
        ),
    ),
    "webui_v2_slack_pairing_ui_regression": CaseSpec(
        name="webui_v2_slack_pairing_ui_regression",
        feature="WebUI v2 Slack proof-code pairing UI",
        category="Hermetic Slack Pairing UI Contract Regression",
        qa_matrix_test_ids=[
            "REBCLI-091-TC-01",
            "REBCLI-091-TC-02",
            "REBCLI-091-TC-03",
            "REBCLI-091-TC-04",
            "REBCLI-091-TC-05",
            "REBCLI-091-TC-06",
            "REBCLI-091-TC-07",
            "REBCLI-091-TC-08",
            "REBCLI-091-TC-09",
            "REBCLI-091-TC-10",
        ],
        commands=[
            REBORN_CLI_WEBUI_V2_BINARY_COMMAND,
            WEBUI_V2_SLACK_PAIRING_UI_COMMAND,
            WEBUI_V2_SLACK_PAIRING_BROWSER_COMMAND,
        ],
        notes=(
            "Covers the generated WebUI v2 Slack proof-code pairing rows at "
            "the static client and browser layers without duplicating PR "
            "#5348 legacy browser scenarios: custom/default copy, blank and "
            "pending disabled states, trimmed button and Enter submissions, "
            "input clearing, query invalidation, success and structured error "
            "messages, Slack-only inbound_proof_code renderer gating, Slack "
            "connect intent routing, authenticated pairing redeem POST body "
            "shape, bearer propagation, and no legacy v1 pairing browser "
            "calls."
        ),
    ),
    "webui_v2_settings_onboarding_client_regression": CaseSpec(
        name="webui_v2_settings_onboarding_client_regression",
        feature="WebUI v2 provider settings and onboarding screens",
        category="Hermetic Settings/Onboarding Client Contract Regression",
        qa_matrix_test_ids=[
            "REBCLI-069-TC-01",
            "REBCLI-069-TC-02",
            "REBCLI-069-TC-03",
            "REBCLI-069-TC-04",
            "REBCLI-069-TC-05",
            "REBCLI-069-TC-06",
            "REBCLI-069-TC-07",
            "REBCLI-069-TC-08",
            "REBCLI-069-TC-09",
        ],
        commands=[
            REBORN_CLI_WEBUI_V2_BINARY_COMMAND,
            WEBUI_V2_SETTINGS_ONBOARDING_CLIENT_COMMAND,
            WEBUI_V2_ONBOARDING_PROVIDER_BROWSER_COMMAND,
        ],
        notes=(
            "Covers the generated WebUI v2 provider settings/onboarding rows "
            "at the static client and committed browser layers without "
            "duplicating PR #5348 settings-search/provider-management "
            "scenarios: provider classification and grouping, setup/dropdown "
            "actions, NEAR AI hosted-SSO localhost guard, wallet and Codex "
            "login recovery, settings v2 LLM/skills/traces/tools API routes, "
            "bearer propagation, users-tab stubs, onboarding-gate redirect "
            "policy, first-run /welcome NEAR AI Google hosted-login "
            "body/origin, Codex device-code UI, and mobile setup-menu "
            "viewport containment."
        ),
    ),
    "webui_v2_hidden_stubbed_routes_regression": CaseSpec(
        name="webui_v2_hidden_stubbed_routes_regression",
        feature="WebUI v2 hidden and stubbed direct routes",
        category="Hermetic Hidden/Stubbed Route Contract Regression",
        qa_matrix_test_ids=[
            "REBCLI-070-TC-01",
            "REBCLI-070-TC-02",
            "REBCLI-070-TC-03",
            "REBCLI-070-TC-04",
            "REBCLI-070-TC-05",
            "REBCLI-070-TC-06",
            "REBCLI-081-TC-01",
            "REBCLI-081-TC-02",
            "REBCLI-081-TC-03",
            "REBCLI-081-TC-04",
            "REBCLI-081-TC-05",
            "REBCLI-081-TC-06",
            "REBCLI-082-TC-01",
            "REBCLI-082-TC-02",
            "REBCLI-082-TC-03",
            "REBCLI-082-TC-04",
            "REBCLI-082-TC-05",
            "REBCLI-082-TC-06",
            "REBCLI-083-TC-01",
            "REBCLI-083-TC-02",
            "REBCLI-083-TC-03",
            "REBCLI-083-TC-04",
            "REBCLI-083-TC-05",
            "REBCLI-083-TC-06",
        ],
        commands=[WEBUI_V2_HIDDEN_STUBBED_ROUTE_COMMAND],
        notes=(
            "Covers the generated WebUI v2 hidden/stubbed direct-route rows "
            "at the static client contract layer: jobs/routines/missions/admin "
            "route metadata stays registered but hidden, routeForId direct "
            "lookup remains available, and jobs/routines/missions/admin API "
            "adapters return empty TODO shapes without calling fetch or "
            "unsupported v1 gateway endpoints. Jobs/routines/missions shell "
            "presenters keep deterministic state labels, action visibility, "
            "sorting, summarization, and duration/id formatting while the "
            "routes remain stubbed. Browser row TC-10 is covered by the "
            "focused Reborn v2 Playwright smoke; browser row TC-11 remains "
            "separate browser/live coverage."
        ),
    ),
    "reborn_cli_trigger_poller_settings_regression": CaseSpec(
        name="reborn_cli_trigger_poller_settings_regression",
        feature="Trigger poller runtime settings",
        category="Hermetic Reborn CLI Runtime Settings Regression",
        qa_matrix_test_ids=[
            "REBCLI-040-TC-01",
            "REBCLI-040-TC-02",
            "REBCLI-040-TC-03",
            "REBCLI-040-TC-04",
            "REBCLI-040-TC-05",
            "REBCLI-040-TC-06",
            "REBCLI-040-TC-07",
        ],
        commands=[REBORN_CLI_TRIGGER_POLLER_SETTINGS_COMMAND],
        notes=(
            "Covers the non-duplicate Reborn CLI runtime settings row that "
            "WebUI v2 scheduled automations depend on: run defaults keep the "
            "poller disabled, serve defaults keep it enabled, config/env "
            "overrides propagate into RuntimeInput, invalid env values fail "
            "closed, and min/max poll intervals are validated before runtime "
            "startup."
        ),
    ),
    "reborn_cli_credential_refresh_settings_regression": CaseSpec(
        name="reborn_cli_credential_refresh_settings_regression",
        feature="Credential refresh worker settings",
        category="Hermetic Reborn CLI Runtime Settings Regression",
        qa_matrix_test_ids=[
            "REBCLI-041-TC-01",
            "REBCLI-041-TC-02",
            "REBCLI-041-TC-03",
            "REBCLI-041-TC-04",
            "REBCLI-041-TC-05",
            "REBCLI-041-TC-06",
            "REBCLI-041-TC-07",
        ],
        commands=[REBORN_CLI_CREDENTIAL_REFRESH_SETTINGS_COMMAND],
        notes=(
            "Covers the non-duplicate Reborn CLI runtime settings row for "
            "the proactive Google OAuth credential refresh worker: run "
            "callers stay disabled by default, serve callers enable refresh "
            "by default, env force-on and kill-switch values propagate into "
            "RuntimeInput, blank env preserves the caller default, and "
            "invalid/non-UTF-8 env values fail closed before runtime startup."
        ),
    ),
    "reborn_cli_docker_railway_entrypoint_regression": CaseSpec(
        name="reborn_cli_docker_railway_entrypoint_regression",
        feature="Docker image and Railway entrypoint",
        category="Hermetic Reborn CLI Deployment Regression",
        qa_matrix_test_ids=[
            "REBCLI-042-TC-01",
            "REBCLI-042-TC-02",
            "REBCLI-042-TC-03",
            "REBCLI-042-TC-04",
            "REBCLI-042-TC-05",
            "REBCLI-042-TC-06",
            "REBCLI-042-TC-07",
        ],
        commands=[
            REBORN_CLI_DOCKERFILE_COMMAND,
            REBORN_CLI_DOCKER_RAILWAY_ENTRYPOINT_COMMAND,
        ],
        notes=(
            "Covers the non-duplicate Reborn CLI deployment row: "
            "Dockerfile.reborn builds the WebUI v2/Slack/libSQL/Postgres "
            "feature binary and ships required configs, and docker/reborn/"
            "entrypoint.sh selects Railway volume homes, rejects unsafe "
            "local-dev Railway storage, accepts production without a volume, "
            "rejects stale local-dev production config, and enforces default "
            "config path safety."
        ),
    ),
    "webui_v2_hidden_workflow_direct_routes_browser_smoke": CaseSpec(
        name="webui_v2_hidden_workflow_direct_routes_browser_smoke",
        feature="WebUI v2 hidden and stubbed direct routes",
        category="Hermetic Reborn v2 Browser Smoke",
        qa_matrix_test_ids=["REBCLI-070-TC-10", "REBCLI-070-TC-11"],
        commands=[
            REBORN_CLI_WEBUI_V2_BINARY_COMMAND,
            WEBUI_V2_HIDDEN_WORKFLOW_BROWSER_COMMAND,
        ],
        notes=(
            "Covers the non-duplicate browser-visible hidden workflow/admin "
            "route rows: starts the Reborn WebUI v2 server against the mock "
            "LLM, drives Chromium to /v2/jobs, /v2/missions, /v2/routines, "
            "and /v2/admin, asserts workflow routes render their empty/TODO "
            "shells, verifies member admin access redirects to chat, verifies "
            "admin bad-tab fallback to /admin/dashboard, and fails if the "
            "browser calls legacy /api/jobs, /api/routines, or "
            "/api/engine/missions endpoints."
        ),
    ),
    "webui_v2_hidden_workflow_presenters_regression": CaseSpec(
        name="webui_v2_hidden_workflow_presenters_regression",
        feature="WebUI v2 hidden workflow screen presenters",
        category="Hermetic Hidden Workflow Presenter Regression",
        qa_matrix_test_ids=[
            "REBCLI-095-TC-01",
            "REBCLI-095-TC-02",
            "REBCLI-095-TC-03",
            "REBCLI-095-TC-04",
            "REBCLI-095-TC-05",
            "REBCLI-095-TC-06",
        ],
        commands=[WEBUI_V2_HIDDEN_STUBBED_ROUTE_COMMAND],
        notes=(
            "Covers hidden Jobs/Missions/Routines presenter rows without "
            "duplicating PR #5348 browser legacy coverage: registered hidden "
            "direct routes, fail-closed TODO adapters with no v1 fetches, "
            "job tab/state/action/date/duration/meta formatting, mission "
            "summary/tone/sort/date fallback behavior, and routine status/"
            "verification/sort/action/date fallback behavior."
        ),
    ),
    "slack_personal_pairing_regression": CaseSpec(
        name="slack_personal_pairing_regression",
        feature="Slack personal pairing workflow",
        category="Hermetic Slack Personal Pairing Regression",
        qa_matrix_test_ids=[
            "REBCLI-053-TC-01",
            "REBCLI-053-TC-02",
            "REBCLI-053-TC-03",
            "REBCLI-053-TC-04",
            "REBCLI-053-TC-05",
            "REBCLI-053-TC-06",
        ],
        commands=[
            SLACK_PERSONAL_PAIRING_ROUTE_COMMAND,
            SLACK_PERSONAL_PAIRING_SERVICE_COMMAND,
        ],
        notes=(
            "Covers Slack personal pairing rows without live Slack network "
            "calls: bearer-bound WebUI proof-code redemption, accepted Slack "
            "channel aliases, invalid/unknown/foreign-tenant/unsupported-code "
            "failures, binding-store unavailable mapping, tenant-scoped "
            "challenge consumption, code validation, pairing challenge issue "
            "failure propagation, resolver challenge issuance, duplicate "
            "cooldown, non-Slack shape skipping, and lookup/issue error "
            "propagation. Browser proof-code button/Enter coverage remains "
            "separate browser/live coverage."
        ),
    ),
    "slack_personal_oauth_binding_regression": CaseSpec(
        name="slack_personal_oauth_binding_regression",
        feature="Slack personal OAuth binding workflow",
        category="Hermetic Slack Personal Binding Regression",
        qa_matrix_test_ids=[
            "REBCLI-071-TC-01",
            "REBCLI-071-TC-02",
            "REBCLI-071-TC-03",
            "REBCLI-071-TC-04",
            "REBCLI-071-TC-05",
            "REBCLI-071-TC-06",
        ],
        commands=[
            SLACK_PERSONAL_BINDING_ROUTE_COMMAND,
            SLACK_PERSONAL_BINDING_SERVICE_COMMAND,
        ],
        notes=(
            "Covers Slack personal OAuth binding rows without live Slack "
            "network calls: protected start descriptor/handler behavior, "
            "Slack authorization URL and callback exchange through mocked "
            "OAuth, sanitized redirect_after handling, single-use/expired "
            "state, denied/missing-code/exchange-failure callback redirects, "
            "binding mismatch/store failure handling, pending-state eviction, "
            "tenant/app/team/installation validation, tenant-app-scope "
            "enforcement, invalid Slack id rejection, and binding-store error "
            "propagation."
        ),
    ),
    "slack_events_ingress_regression": CaseSpec(
        name="slack_events_ingress_regression",
        feature="Slack Events host ingress workflow",
        category="Hermetic Slack Events Ingress Regression",
        qa_matrix_test_ids=[
            "REBCLI-052-TC-01",
            "REBCLI-052-TC-02",
            "REBCLI-052-TC-03",
            "REBCLI-052-TC-04",
            "REBCLI-052-TC-05",
            "REBCLI-052-TC-06",
            "REBCLI-052-TC-07",
            "REBCLI-052-TC-08",
        ],
        commands=[
            SLACK_EVENTS_INGRESS_COMMAND,
            SLACK_HOST_BETA_CLI_SERVE_COMMAND,
        ],
        notes=(
            "Covers Slack Events host ingress rows without live Slack network "
            "calls: URL verification, signed event dispatch, malformed "
            "envelopes, missing/ambiguous installation rejection, per-install "
            "rate limiting, adapter panic/timeout response mapping, route "
            "descriptor body/rate policy, e2e native ProductAdapter flow, "
            "and env-enabled serve route mounting."
        ),
    ),
    "slack_shared_channel_admin_regression": CaseSpec(
        name="slack_shared_channel_admin_regression",
        feature="Slack shared-channel admin and target workflow",
        category="Hermetic Slack Shared-Channel Admin Regression",
        qa_matrix_test_ids=[
            "REBCLI-054-TC-01",
            "REBCLI-054-TC-02",
            "REBCLI-054-TC-03",
            "REBCLI-054-TC-04",
            "REBCLI-054-TC-05",
            "REBCLI-054-TC-06",
            "REBCLI-054-TC-07",
            "REBCLI-054-TC-08",
            "REBCLI-054-TC-09",
            "REBCLI-054-TC-10",
            "REBCLI-054-TC-11",
        ],
        commands=[
            SLACK_SHARED_CHANNEL_ADMIN_COMMAND,
            WEBUI_V2_SLACK_CHANNEL_ADMIN_CLIENT_COMMAND,
        ],
        notes=(
            "Covers Slack shared-channel admin rows without live Slack "
            "network calls: WebUI channel route list/upsert/delete, "
            "operator-only and cross-tenant gating, dynamic/static route "
            "merging, route owner changes, outbound target authority updates, "
            "invalid/duplicate channel validation, client allowed-channel "
            "normalization/save/list payloads, subject preservation, setup "
            "dirty-field protection, and picker error states."
        ),
    ),
    "slack_host_beta_serve_mount_regression": CaseSpec(
        name="slack_host_beta_serve_mount_regression",
        feature="Slack host-beta serve mount",
        category="Hermetic Slack Host-Beta Serve Mount Regression",
        qa_matrix_test_ids=[
            "REBCLI-038-TC-01",
            "REBCLI-038-TC-02",
            "REBCLI-038-TC-03",
            "REBCLI-038-TC-04",
            "REBCLI-038-TC-05",
            "REBCLI-038-TC-06",
            "REBCLI-038-TC-07",
            "REBCLI-038-TC-08",
        ],
        commands=[
            SLACK_HOST_BETA_WEBUI_ONLY_CLI_COMMAND,
            SLACK_HOST_BETA_CLI_SERVE_COMMAND,
            SLACK_HOST_BETA_COMPOSITION_COMMAND,
        ],
        notes=(
            "Covers Slack host-beta serve mount rows without live Slack "
            "network calls or PR #5348 browser duplication: WebUI-only "
            "feature-disabled fail-closed behavior, env-enabled serve route "
            "mounting, Slack Events API descriptor/handler behavior, signed "
            "URL verification/event dispatch, pairing redeem exposure, channel "
            "routing, and runtime dependency failures."
        ),
    ),
    "slack_outbound_delivery_rendering_regression": CaseSpec(
        name="slack_outbound_delivery_rendering_regression",
        feature="Slack outbound delivery, rendering, and DM targets",
        category="Hermetic Slack Outbound Delivery Regression",
        qa_matrix_test_ids=[
            "REBCLI-072-TC-01",
            "REBCLI-072-TC-02",
            "REBCLI-072-TC-03",
            "REBCLI-072-TC-04",
            "REBCLI-072-TC-05",
            "REBCLI-072-TC-06",
        ],
        commands=[
            SLACK_DELIVERY_COMMAND,
            SLACK_EGRESS_COMMAND,
            SLACK_OUTBOUND_TARGETS_COMMAND,
            SLACK_DM_OPEN_COMMAND,
            SLACK_ADAPTER_COMMAND,
        ],
        notes=(
            "Covers Slack outbound delivery rows without live Slack network "
            "calls: final reply delivery, long reply chunking, mrkdwn "
            "rendering, approval/auth prompt rendering, busy/timeout/error "
            "messages, duplicate/retry suppression, delivery permits and caps, "
            "personal DM open/list/resolve, shared-channel target resolution, "
            "host-mediated HTTPS egress policy, opaque credential-handle bearer "
            "injection, and token-safe Slack ok:false handling."
        ),
    ),
    "webui_v2_logs_screen_regression": CaseSpec(
        name="webui_v2_logs_screen_regression",
        feature="WebUI v2 logs screen and scoped log filters",
        category="Hermetic Logs Screen Client/API Regression",
        qa_matrix_test_ids=[
            "REBCLI-073-TC-01",
            "REBCLI-073-TC-02",
            "REBCLI-073-TC-03",
            "REBCLI-073-TC-04",
            "REBCLI-073-TC-05",
            "REBCLI-073-TC-06",
            "REBCLI-073-TC-07",
        ],
        commands=[
            REBORN_CLI_WEBUI_V2_BINARY_COMMAND,
            WEBUI_V2_LOGS_CLIENT_COMMAND,
            WEBUI_V2_OPERATOR_LOGS_HANDLER_COMMAND,
            WEBUI_V2_LOGS_BROWSER_COMMAND,
        ],
        notes=(
            "Covers the WebUI v2 logs rows, including the browser-visible "
            "scoped logs smoke, without duplicating legacy v1 routes: scoped "
            "query normalization, "
            "public/operator log fallback behavior, paused scope reloads, stale "
            "entry suppression, unsupported operator-route handling, empty/error "
            "states, scroll layout, chat duplicate-log-bar suppression, "
            "automation recent-run log links, operator logs capability "
            "enforcement, and Chromium rendering of scoped log context."
        ),
    ),
    "webui_v2_shell_navigation_regression": CaseSpec(
        name="webui_v2_shell_navigation_regression",
        feature="WebUI v2 global shell, navigation, and session controls",
        category="Hermetic Shell Navigation Client Regression",
        qa_matrix_test_ids=[
            "REBCLI-074-TC-01",
            "REBCLI-074-TC-02",
            "REBCLI-074-TC-03",
            "REBCLI-074-TC-04",
            "REBCLI-074-TC-05",
            "REBCLI-074-TC-06",
            "REBCLI-074-TC-07",
        ],
        commands=[
            REBORN_CLI_WEBUI_V2_BINARY_COMMAND,
            WEBUI_V2_SHELL_CLIENT_COMMAND,
            WEBUI_V2_SHELL_BROWSER_COMMAND,
        ],
        notes=(
            "Covers the WebUI v2 shell/navigation rows: first-run onboarding "
            "redirects, responsive sidebar state, command palette actions and "
            "thread jumps, admin/settings route filtering, explicit thread "
            "pin/search/delete handling, account popover, theme/sign-out "
            "controls, header logs/docs/TEE affordances, toasts, "
            "thread-delete error messaging, and a Chromium smoke for command "
            "palette route jumps plus sidebar navigation/collapse."
        ),
    ),
    "webui_v2_frontend_bundle_supply_chain_regression": CaseSpec(
        name="webui_v2_frontend_bundle_supply_chain_regression",
        feature="WebUI v2 frontend bundle build and dependency supply chain",
        category="Hermetic Frontend Build/Supply-Chain Regression",
        qa_matrix_test_ids=[
            "REBCLI-075-TC-01",
            "REBCLI-075-TC-02",
            "REBCLI-075-TC-03",
            "REBCLI-075-TC-04",
            "REBCLI-075-TC-05",
            "REBCLI-075-TC-06",
        ],
        commands=[
            WEBUI_V2_FRONTEND_BUILD_COMMAND,
            WEBUI_V2_STATIC_JS_COMMAND,
            WEBUI_V2_RUST_STATIC_COMMAND,
            WEBUI_V2_COMPOSITION_STATIC_COMMAND,
        ],
        notes=(
            "Covers the WebUI v2 frontend build and dependency supply-chain "
            "rows: npm ci package-lock consistency, high-severity npm audit "
            "gate, no-vendor esbuild bundle rebuild, committed app/chunk output "
            "shape, static JS suite, embedded static asset/router tests, and "
            "composition static route contracts. This is not browser/live "
            "coverage and does not duplicate PR #5348."
        ),
    ),
    "webui_v2_i18n_language_regression": CaseSpec(
        name="webui_v2_i18n_language_regression",
        feature="WebUI v2 internationalization and language selection",
        category="Hermetic I18n/Language Client Regression",
        qa_matrix_test_ids=[
            "REBCLI-087-TC-01",
            "REBCLI-087-TC-02",
            "REBCLI-087-TC-03",
            "REBCLI-087-TC-04",
            "REBCLI-087-TC-05",
            "REBCLI-087-TC-06",
        ],
        commands=[WEBUI_V2_I18N_LANGUAGE_COMMAND],
        notes=(
            "Covers WebUI v2 i18n/language-selection rows without duplicating "
            "PR #5348 browser settings coverage: saved/navigator/default "
            "language detection, lazy pack success, concurrent import "
            "memoization, failed import retryability, stale-load protection, "
            "translation fallback, locale pack key presence, current-language "
            "display, search filtering, setLang routing, and empty-search "
            "rendering."
        ),
    ),
    "webui_v2_settings_shell_role_gating_regression": CaseSpec(
        name="webui_v2_settings_shell_role_gating_regression",
        feature="WebUI v2 settings shell navigation and role gating",
        category="Hermetic Settings Shell Role-Gating Regression",
        qa_matrix_test_ids=[
            "REBCLI-088-TC-01",
            "REBCLI-088-TC-02",
            "REBCLI-088-TC-03",
            "REBCLI-088-TC-04",
            "REBCLI-088-TC-05",
            "REBCLI-088-TC-06",
        ],
        commands=[WEBUI_V2_SETTINGS_SHELL_COMMAND],
        notes=(
            "Covers WebUI v2 Settings shell/navigation rows without "
            "duplicating PR #5348 browser settings coverage: admin/member "
            "default tabs, unknown-tab redirects, non-admin operator-tab "
            "redirects, desktop role filtering, admin tab exposure, mobile "
            "hidden-active fallback, and tab-click callbacks."
        ),
    ),
    "webui_v2_settings_restart_banner_regression": CaseSpec(
        name="webui_v2_settings_restart_banner_regression",
        feature="WebUI v2 settings restart availability banner",
        category="Hermetic Settings Restart Banner Regression",
        qa_matrix_test_ids=[
            "REBCLI-089-TC-01",
            "REBCLI-089-TC-02",
            "REBCLI-089-TC-03",
            "REBCLI-089-TC-04",
            "REBCLI-089-TC-05",
            "REBCLI-089-TC-06",
        ],
        commands=[WEBUI_V2_SETTINGS_RESTART_COMMAND],
        notes=(
            "Covers WebUI v2 Settings restart banner rows without adding a "
            "legacy restart implementation: no banner when needsRestart is "
            "false, banner rendering when needsRestart is true, disabled "
            "restart interface, unavailable reason, local confirmation "
            "callbacks, and no v1 restart side effects."
        ),
    ),
    "webui_v2_settings_toolbar_search_regression": CaseSpec(
        name="webui_v2_settings_toolbar_search_regression",
        feature="WebUI v2 settings search and JSON import/export toolbar",
        category="Hermetic Settings Toolbar/Search Regression",
        qa_matrix_test_ids=[
            "REBCLI-090-TC-01",
            "REBCLI-090-TC-02",
            "REBCLI-090-TC-03",
            "REBCLI-090-TC-04",
            "REBCLI-090-TC-05",
            "REBCLI-090-TC-06",
        ],
        commands=[WEBUI_V2_SETTINGS_TOOLBAR_SEARCH_COMMAND],
        notes=(
            "Covers WebUI v2 Settings toolbar/search rows without duplicating "
            "PR #5348 browser settings coverage: toolbar reachability from "
            "SettingsPage, search change/clear wiring, JSON export payload "
            "shape, valid import dispatch, invalid import rejection, empty "
            "file handling, settings search matching, and v2 settings API "
            "route selection."
        ),
    ),
    "webui_v2_settings_direct_tabs_regression": CaseSpec(
        name="webui_v2_settings_direct_tabs_regression",
        feature="WebUI v2 settings direct tabs and configuration panels",
        category="Hermetic Settings Direct Tabs/Configuration Panel Regression",
        qa_matrix_test_ids=[
            "REBCLI-096-TC-01",
            "REBCLI-096-TC-02",
            "REBCLI-096-TC-03",
            "REBCLI-096-TC-04",
            "REBCLI-096-TC-05",
            "REBCLI-096-TC-06",
        ],
        commands=[WEBUI_V2_SETTINGS_DIRECT_TABS_COMMAND],
        notes=(
            "Covers WebUI v2 Settings direct-tab/configuration panel rows "
            "without duplicating PR #5348 browser settings/tool-permission "
            "coverage: direct /settings/:tab dispatch, role-based redirects, "
            "desktop/mobile tab visibility, toolbar and restart wiring, "
            "schema restart rules, channel grouping/search/empty states, "
            "tool permission controls, users forbidden/error/list/search "
            "states, and v2 settings API route selection."
        ),
    ),
    "webui_v2_admin_console_usage_regression": CaseSpec(
        name="webui_v2_admin_console_usage_regression",
        feature="WebUI v2 admin console and usage presentation",
        category="Hermetic Admin Console/Usage Client Regression",
        qa_matrix_test_ids=[
            "REBCLI-093-TC-01",
            "REBCLI-093-TC-02",
            "REBCLI-093-TC-03",
            "REBCLI-093-TC-04",
            "REBCLI-093-TC-05",
            "REBCLI-093-TC-06",
        ],
        commands=[WEBUI_V2_ADMIN_CLIENT_COMMAND],
        notes=(
            "Covers WebUI v2 Admin console rows without duplicating PR #5348 "
            "browser legacy coverage: dashboard/users/usage routing, user "
            "drilldown handoff, desktop/mobile admin tab navigation, fail-"
            "closed v2 TODO API stubs with no legacy v1 fetches, and usage/"
            "user presenter formatting, filtering, summarization, and cost-"
            "sorted aggregation."
        ),
    ),
    "webui_v2_toast_query_defaults_regression": CaseSpec(
        name="webui_v2_toast_query_defaults_regression",
        feature="WebUI v2 toast notifications and query cache defaults",
        category="Hermetic Toast/Query Client Regression",
        qa_matrix_test_ids=[
            "REBCLI-094-TC-01",
            "REBCLI-094-TC-02",
            "REBCLI-094-TC-03",
            "REBCLI-094-TC-04",
            "REBCLI-094-TC-05",
            "REBCLI-094-TC-06",
        ],
        commands=[WEBUI_V2_TOAST_QUERY_CLIENT_COMMAND],
        notes=(
            "Covers WebUI v2 toast/query-cache rows without duplicating PR "
            "#5348 browser legacy coverage: toast publication defaults and "
            "overrides, multi-subscriber delivery, unsubscribe behavior, "
            "ToastViewport status rendering, auto-removal, unknown-tone "
            "fallbacks, GatewayLayout mounting, and QueryClient retry/"
            "staleTime/refetchOnWindowFocus defaults."
        ),
    ),
    "webui_v2_tee_attestation_regression": CaseSpec(
        name="webui_v2_tee_attestation_regression",
        feature="WebUI v2 TEE attestation indicator and report copy",
        category="Hermetic TEE Attestation Client Regression",
        qa_matrix_test_ids=[
            "REBCLI-076-TC-01",
            "REBCLI-076-TC-02",
            "REBCLI-076-TC-03",
            "REBCLI-076-TC-04",
            "REBCLI-076-TC-05",
            "REBCLI-076-TC-06",
        ],
        commands=[WEBUI_V2_TEE_CLIENT_COMMAND],
        notes=(
            "Covers WebUI v2 TEE attestation rows at the static client "
            "contract layer: deployment-owned API endpoint derivation, "
            "localhost/IP suppression, encoded instance attestation fetch, "
            "on-demand report fetch with reuse/error state, clipboard payload "
            "formatting and no-clipboard gating, hidden shield unavailable "
            "state, loading/error/copy UI states, and PageHeader integration. "
            "Live enclave evidence remains outside this hermetic lane."
        ),
    ),
    "webui_v2_sidebar_trace_credits_regression": CaseSpec(
        name="webui_v2_sidebar_trace_credits_regression",
        feature="WebUI v2 sidebar Trace Commons credits card",
        category="Hermetic Trace Credits Sidebar Client Regression",
        qa_matrix_test_ids=[
            "REBCLI-077-TC-01",
            "REBCLI-077-TC-02",
            "REBCLI-077-TC-03",
            "REBCLI-077-TC-04",
            "REBCLI-077-TC-05",
            "REBCLI-077-TC-06",
        ],
        commands=[
            REBORN_CLI_WEBUI_V2_BINARY_COMMAND,
            WEBUI_V2_TRACE_CREDITS_CLIENT_COMMAND,
            WEBUI_V2_TRACE_CREDITS_BROWSER_COMMAND,
        ],
        notes=(
            "Covers the WebUI v2 sidebar Trace Commons credits card at the "
            "static client contract and served browser layers: hidden "
            "loading/error/not-enrolled states, signed two-decimal final-credit "
            "formatting, accepted and submitted defaults, positive held-count "
            "visibility, settings/traces navigation, shared trace-credits "
            "react-query key, display-only sidebar placement, bearer-backed "
            "credit fetch, and real SPA routing. Live Trace Commons ledger/API "
            "behavior remains outside this hermetic lane."
        ),
    ),
    "webui_v2_wallet_connect_regression": CaseSpec(
        name="webui_v2_wallet_connect_regression",
        feature="WebUI v2 NEAR wallet connect popup",
        category="Hermetic Wallet Connect Popup Regression",
        qa_matrix_test_ids=[
            "REBCLI-078-TC-01",
            "REBCLI-078-TC-02",
            "REBCLI-078-TC-03",
            "REBCLI-078-TC-04",
            "REBCLI-078-TC-05",
            "REBCLI-078-TC-06",
        ],
        commands=[
            REBORN_CLI_WEBUI_V2_BINARY_COMMAND,
            WEBUI_V2_WALLET_CONNECT_CLIENT_COMMAND,
            WEBUI_V2_WALLET_CONNECT_ROUTER_COMMAND,
            WEBUI_V2_LLM_PROVIDER_ROUTE_COMMAND,
            WEBUI_V2_WALLET_CONNECT_BROWSER_COMMAND,
        ],
        notes=(
            "Covers the WebUI v2 NEAR wallet connect popup without live wallet "
            "interaction: static popup route, missing channel/BroadcastChannel "
            "fail-closed source path, epoch-millis plus random nonce layout, "
            "fixed NEAR AI message/recipient, BroadcastChannel success/failure "
            "payloads, no-store and wallet-scoped relaxed CSP, strict SPA CSP "
            "isolation, protected backend wallet completion route gating, and "
            "a real-browser served-popup smoke that stubs the remote connector "
            "module and observes the signed success payload. Live "
            "wallet-provider behavior remains external/canary scope."
        ),
    ),
    "reborn_operator_logs_service_regression": CaseSpec(
        name="reborn_operator_logs_service_regression",
        feature="Reborn operator log buffer and correlation query service",
        category="Hermetic Operator Logs Service Regression",
        qa_matrix_test_ids=[
            "REBCLI-079-TC-01",
            "REBCLI-079-TC-02",
            "REBCLI-079-TC-03",
            "REBCLI-079-TC-04",
            "REBCLI-079-TC-05",
            "REBCLI-079-TC-06",
        ],
        commands=[
            REBORN_OPERATOR_LOGS_SERVICE_COMMAND,
            WEBUI_V2_OPERATOR_LOGS_HANDLER_COMMAND,
            WEBUI_V2_OPERATOR_LOGS_ROUTE_DISPATCH_COMMAND,
            WEBUI_V2_OPERATOR_LOGS_BROWSER_COMMAND,
        ],
        notes=(
            "Covers the Reborn/WebUI v2 operator log buffer rows without "
            "duplicating the existing browser log-screen scenario: in-memory "
            "ring buffer retention, newest-first and before-cursor pagination, "
            "invalid cursor behavior, level/target/correlation filtering, "
            "alias precedence, tracing span/event capture, arbitrary-field "
            "exclusion from stored correlation, secret/path redaction, UTF-8 "
            "message truncation, response byte caps, tail/follow cursors, "
            "operator route capability plus dispatch contracts, and served "
            "WebUIv2 logs-page scope propagation/context rendering."
        ),
    ),
    "webui_v2_filesystem_api_regression": CaseSpec(
        name="webui_v2_filesystem_api_regression",
        feature="WebUI v2 workspace filesystem API",
        category="Hermetic Rust/API",
        qa_matrix_test_ids=["REBCLI-084-TC-08"],
        commands=[
            WEBUI_V2_FS_HANDLER_COMMAND,
            COMPOSITION_MOUNT_FS_COMMAND,
            WEBUI_V2_HANDLER_CONTRACT_COMMAND,
        ],
        notes=(
            "Runs the focused WebUI v2 filesystem handler slice, composition "
            "mount filesystem reader policy tests, and full handler contract file."
        ),
    ),
    "webui_v2_project_files_api_regression": CaseSpec(
        name="webui_v2_project_files_api_regression",
        feature="WebUI v2 project file and filesystem browser APIs",
        category="Hermetic Project Filesystem API Regression",
        qa_matrix_test_ids=[
            "REBCLI-049-TC-01",
            "REBCLI-049-TC-02",
            "REBCLI-049-TC-03",
            "REBCLI-049-TC-04",
            "REBCLI-049-TC-05",
            "REBCLI-049-TC-06",
        ],
        commands=[
            WEBUI_V2_FS_HANDLER_COMMAND,
            COMPOSITION_PROJECT_FS_COMMAND,
            COMPOSITION_MOUNT_FS_COMMAND,
        ],
        notes=(
            "Covers project-file and read-only filesystem API rows: fs route "
            "mount/list/stat/read handlers, project-scoped reader confinement, "
            "hidden/sensitive path denial, oversize and missing-file handling, "
            "mount-relative traversal rejection, and attachment download "
            "headers without duplicating browser file-tree smoke coverage."
        ),
    ),
    "webui_v2_project_membership_api_regression": CaseSpec(
        name="webui_v2_project_membership_api_regression",
        feature="WebUI v2 project and membership APIs",
        category="Hermetic Project/Membership API Regression",
        qa_matrix_test_ids=[
            "REBCLI-050-TC-01",
            "REBCLI-050-TC-02",
            "REBCLI-050-TC-03",
            "REBCLI-050-TC-04",
            "REBCLI-050-TC-05",
            "REBCLI-050-TC-06",
            "REBCLI-050-TC-07",
            "REBCLI-080-TC-01",
            "REBCLI-080-TC-02",
            "REBCLI-080-TC-03",
            "REBCLI-080-TC-04",
            "REBCLI-080-TC-05",
            "REBCLI-080-TC-06",
        ],
        commands=[
            WEBUI_V2_DESCRIPTOR_POLICY_COMMAND,
            WEBUI_V2_PROJECT_HANDLER_COMMAND,
            WEBUI_V2_PROJECTS_HANDLER_COMMAND,
            WEBUI_V2_MEMBER_HANDLER_COMMAND,
            WEBUI_V2_PROJECTS_CLIENT_API_COMMAND,
            COMPOSITION_PROJECT_SERVICE_COMMAND,
        ],
        notes=(
            "Covers WebUI v2 project and membership API rows without "
            "duplicating PR #5348 browser project overview coverage: "
            "descriptor policy, project collection and item routes, project "
            "client API mapping and route encoding, path/body ID precedence, "
            "member add/update/remove routing, unwired service fail-closed "
            "behavior, no-content delete responses, reborn-projects session "
            "feature projection, and project service authorization contracts."
        ),
    ),
    "webui_v2_public_sso_session_regression": CaseSpec(
        name="webui_v2_public_sso_session_regression",
        feature="WebUI v2 public SSO session routes",
        category="Hermetic Public SSO Session Regression",
        qa_matrix_test_ids=[
            "REBCLI-051-TC-01",
            "REBCLI-051-TC-02",
            "REBCLI-051-TC-03",
            "REBCLI-051-TC-04",
            "REBCLI-051-TC-05",
            "REBCLI-051-TC-06",
            "REBCLI-051-TC-07",
            "REBCLI-051-TC-08",
        ],
        commands=[
            WEBUI_V2_SSO_AUTH_ROUTE_COMMAND,
            WEBUI_V2_GOOGLE_OAUTH_ROUTE_COMMAND,
            WEBUI_V2_GITHUB_OAUTH_ROUTE_COMMAND,
            WEBUI_V2_SESSION_ROUND_TRIP_COMMAND,
            WEBUI_V2_SSO_NETWORK_LIMITS_COMMAND,
            WEBUI_V2_SSO_PUBLIC_MOUNT_COMMAND,
            WEBUI_V2_PUBLIC_SSO_OWNER_CRATE_COMMAND,
            REBORN_IDENTITY_FOUNDATION_COMMAND,
        ],
        notes=(
            "Covers WebUI v2 public SSO session rows without live provider "
            "calls: provider discovery, Google/GitHub login redirect and "
            "callback success/failure, one-time state/ticket replay guards, "
            "session bearer use on protected WebUI v2 routes, logout "
            "revocation, public route mount policy, open-redirect defense, "
            "body/rate limits, CORS fail-closed behavior, sanitized errors, "
            "the full WebUI ingress owner-crate SSO/session regression, and "
            "the Reborn identity foundation mapping layer."
        ),
    ),
    "webui_v2_rust_static_regression": CaseSpec(
        name="webui_v2_rust_static_regression",
        feature="WebUI v2 native routes and static router",
        category="WebUI Rust/Static Regression",
        qa_matrix_test_ids=["REBCLI-055-TC-13"],
        commands=[WEBUI_V2_RUST_STATIC_COMMAND],
        notes=(
            "Matches the QA matrix Rust/static command for REBCLI-055-TC-13; "
            "browser/static Node coverage remains separate."
        ),
    ),
    "webui_v2_composition_regression": CaseSpec(
        name="webui_v2_composition_regression",
        feature="CLI-served WebUI v2 gateway composition",
        category="WebUI Composition Regression",
        qa_matrix_test_ids=["REBCLI-055-TC-09"],
        commands=[WEBUI_V2_COMPOSITION_COMMAND],
        notes=(
            "Matches the QA matrix composition command for REBCLI-055-TC-09; "
            "this validates the Rust gateway composition layer rather than "
            "duplicating browser coverage from PR #5348."
        ),
    ),
    "webui_v2_spa_static_serving_regression": CaseSpec(
        name="webui_v2_spa_static_serving_regression",
        feature="WebUI v2 SPA shell and static asset serving",
        category="Hermetic WebUI v2 Static Serving Regression",
        qa_matrix_test_ids=[
            "REBCLI-063-TC-01",
            "REBCLI-063-TC-02",
            "REBCLI-063-TC-03",
            "REBCLI-063-TC-04",
            "REBCLI-063-TC-05",
            "REBCLI-063-TC-06",
        ],
        commands=[
            WEBUI_V2_STATIC_ROUTER_COMMAND,
            WEBUI_V2_COMPOSITION_STATIC_COMMAND,
        ],
        notes=(
            "Covers WebUI v2 SPA/static route rows without duplicating PR "
            "#5348 browser shell coverage: /v2 root and direct client-route "
            "fallbacks, JS/CSS asset content types, unknown asset 404s, path "
            "traversal rejection, no-bearer static root access, fresh matching "
            "CSP nonces, no-store shell responses, locked document CSP "
            "allowlists, wallet-connect CSP isolation, static security "
            "headers, and mount prefix validation."
        ),
    ),
    "webui_v2_login_session_state_regression": CaseSpec(
        name="webui_v2_login_session_state_regression",
        feature="WebUI v2 login and client session state",
        category="Hermetic Login/Session State Regression",
        qa_matrix_test_ids=[
            "REBCLI-064-TC-01",
            "REBCLI-064-TC-02",
            "REBCLI-064-TC-03",
            "REBCLI-064-TC-04",
            "REBCLI-064-TC-05",
            "REBCLI-064-TC-06",
            "REBCLI-064-TC-07",
            "REBCLI-064-TC-08",
            "REBCLI-064-TC-09",
            "REBCLI-064-TC-10",
            "REBCLI-064-TC-11",
            "REBCLI-064-TC-12",
            "REBCLI-064-TC-13",
            "REBCLI-064-TC-14",
            "REBCLI-064-TC-15",
            "REBCLI-064-TC-16",
            "REBCLI-064-TC-17",
            "REBCLI-064-TC-18",
            "REBCLI-085-TC-01",
            "REBCLI-085-TC-02",
            "REBCLI-085-TC-03",
            "REBCLI-085-TC-04",
            "REBCLI-085-TC-05",
            "REBCLI-085-TC-06",
            "REBCLI-086-TC-01",
            "REBCLI-086-TC-02",
            "REBCLI-086-TC-03",
            "REBCLI-086-TC-04",
            "REBCLI-086-TC-05",
            "REBCLI-086-TC-06",
        ],
        commands=[
            WEBUI_V2_STATIC_AUTH_JS_COMMAND,
            WEBUI_V2_STATIC_API_AUTH_COMMAND,
            WEBUI_V2_LOGIN_OAUTH_CLIENT_COMMAND,
            WEBUI_V2_LOGIN_BROWSER_MATRIX_COMMAND,
            WEBUI_V2_INGRESS_SESSION_AUTH_COMMAND,
        ],
        notes=(
            "Covers WebUI v2 login/client-session rows without duplicating PR "
            "#5348 browser auth-flow coverage: login-ticket consumption, URL "
            "credential stripping, stored-token non-overwrite, logout revoke "
            "dispatch, login_error handling, sessionStorage bearer use on "
            "same-origin API calls, off-origin bearer-send prevention, env "
            "bearer matching, session creation/lookup/expiry, one-time "
            "tickets, revoked-session denial, tenant isolation, signed "
            "session round-trips, protected-route authentication, public OAuth "
            "provider discovery, provider ordering/filtering, discovery "
            "failure behavior, encoded login button href construction, and "
            "browser-visible manual-token, OAuth-ticket, sign-out, mobile, "
            "token-scrubbing, stored-token, fragment-precedence, and "
            "login_error workflows."
        ),
    ),
    "webui_v2_product_auth_oauth_regression": CaseSpec(
        name="webui_v2_product_auth_oauth_regression",
        feature="WebUI v2 product-auth OAuth start and callback routes",
        category="Hermetic Product Auth OAuth Regression",
        qa_matrix_test_ids=[
            "REBCLI-059-TC-01",
            "REBCLI-059-TC-02",
            "REBCLI-059-TC-03",
            "REBCLI-059-TC-04",
            "REBCLI-059-TC-05",
            "REBCLI-059-TC-06",
            "REBCLI-059-TC-07",
        ],
        commands=[
            WEBUI_V2_PRODUCT_AUTH_OAUTH_COMMAND,
            WEBUI_V2_PRODUCT_AUTH_GOOGLE_OAUTH_COMMAND,
            WEBUI_V2_PRODUCT_AUTH_CALLBACK_COMMAND,
            WEBUI_V2_PRODUCT_AUTH_SERVICE_SUBSTRATE_COMMAND,
        ],
        notes=(
            "Covers WebUI v2 product-auth OAuth API/runtime rows without "
            "duplicating PR #5348 browser auth-card coverage: generic and "
            "Google OAuth start/callback success, browser completion without "
            "secret leakage, provider denial/exchange failure, malformed and "
            "unknown callback state, invalid scope/expiry rejection, "
            "cross-scope rejection, bearer/no-body enforcement, and "
            "per-caller/per-IP rate limits. Also runs the product-auth "
            "service-substrate sweep for auth/OAuth flow state, provider "
            "exchange and refresh boundaries, workflow gates, adapters, "
            "outbound, triggers, projects, and conversations without live "
            "provider credentials."
        ),
    ),
    "webui_v2_extension_oauth_setup_regression": CaseSpec(
        name="webui_v2_extension_oauth_setup_regression",
        feature="WebUI v2 extension OAuth setup routes",
        category="Hermetic Extension OAuth Setup Regression",
        qa_matrix_test_ids=[
            "REBCLI-060-TC-01",
            "REBCLI-060-TC-02",
            "REBCLI-060-TC-03",
            "REBCLI-060-TC-04",
            "REBCLI-060-TC-05",
            "REBCLI-060-TC-06",
        ],
        commands=[
            WEBUI_V2_EXTENSION_OAUTH_ROUTE_COMMAND,
            WEBUI_V2_EXTENSION_OAUTH_START_COMMAND,
            WEBUI_V2_EXTENSION_GOOGLE_OAUTH_COMMAND,
            WEBUI_V2_DCR_OAUTH_CALLBACK_COMMAND,
        ],
        notes=(
            "Covers WebUI v2 extension OAuth setup API/runtime rows without "
            "duplicating PR #5348 browser extension/auth-flow coverage: "
            "package-scoped setup route binding, Google extension OAuth start, "
            "DCR extension OAuth start, existing-owner reconnect binding, "
            "cross-owner binding rejection, missing DCR registry fail-closed "
            "behavior, binding lookup fallback, DCR callback state/PKCE "
            "fallback, and blocked-turn gate resume."
        ),
    ),
    "webui_v2_manual_token_regression": CaseSpec(
        name="webui_v2_manual_token_regression",
        feature="WebUI v2 product-auth manual-token routes",
        category="Hermetic Manual Token Regression",
        qa_matrix_test_ids=[
            "REBCLI-061-TC-01",
            "REBCLI-061-TC-02",
            "REBCLI-061-TC-03",
            "REBCLI-061-TC-04",
            "REBCLI-061-TC-05",
            "REBCLI-061-TC-06",
            "REBCLI-061-TC-08",
        ],
        commands=[
            WEBUI_V2_MANUAL_TOKEN_LEGACY_COMMAND,
            WEBUI_V2_MANUAL_TOKEN_SPLIT_COMMAND,
            WEBUI_V2_MANUAL_TOKEN_FACADE_COMMAND,
            WEBUI_V2_MANUAL_TOKEN_POSTGRES_MIGRATION_FACADE_COMMAND,
            WEBUI_V2_MANUAL_TOKEN_POSTGRES_FACADE_COMMAND,
            WEBUI_V2_MANUAL_TOKEN_LIBSQL_FACADE_COMMAND,
        ],
        notes=(
            "Covers WebUI v2 manual-token API/runtime rows without "
            "duplicating PR #5348 browser auth-card coverage: legacy submit "
            "success and redaction, split setup/secret-submit success, seeded "
            "gate projection, invalid secret redaction, abandoned interactions "
            "on submit failure, partial continuation rejection, missing "
            "invocation enforcement, bearer/body/rate-limit enforcement, "
            "facade retry/cross-scope/fail-closed behavior, and sanitized "
            "backend failures. Also runs feature-gated facade_factory "
            "Postgres/libSQL contracts for migration-dry-run process-port "
            "coverage and durable local-dev manual-token setup parity."
        ),
    ),
    "webui_v2_product_auth_account_lifecycle_regression": CaseSpec(
        name="webui_v2_product_auth_account_lifecycle_regression",
        feature="WebUI v2 product-auth account and lifecycle routes",
        category="Hermetic Product Auth Account/Lifecycle Regression",
        qa_matrix_test_ids=[
            "REBCLI-062-TC-01",
            "REBCLI-062-TC-02",
            "REBCLI-062-TC-03",
            "REBCLI-062-TC-04",
            "REBCLI-062-TC-05",
            "REBCLI-062-TC-06",
        ],
        commands=[
            WEBUI_V2_ACCOUNT_ROUTE_COMMAND,
            WEBUI_V2_LIFECYCLE_CLEANUP_COMMAND,
        ],
        notes=(
            "Covers WebUI v2 product-auth account/lifecycle API/runtime rows "
            "without duplicating PR #5348 browser auth-flow coverage: account "
            "listing, selection, recovery projections, refresh behavior, "
            "redacted projections, malformed and unknown account ids, "
            "wrong-provider, foreign-scope, and unconfigured account handling, "
            "missing invocation ids, refresh rate limits, lifecycle cleanup "
            "dispatch, invalid extension rejection, and secret-free responses."
        ),
    ),
    "webui_v2_operator_config_api_regression": CaseSpec(
        name="webui_v2_operator_config_api_regression",
        feature="WebUI v2 LLM and operator configuration APIs",
        category="Hermetic Operator Configuration API Regression",
        qa_matrix_test_ids=[
            "REBCLI-048-TC-01",
            "REBCLI-048-TC-02",
            "REBCLI-048-TC-03",
            "REBCLI-048-TC-04",
            "REBCLI-048-TC-05",
            "REBCLI-048-TC-06",
            "REBCLI-048-TC-07",
        ],
        commands=[
            WEBUI_V2_DESCRIPTOR_POLICY_COMMAND,
            WEBUI_V2_LLM_PROVIDER_ROUTE_COMMAND,
            IRONCLAW_LLM_PROVIDER_SUBSTRATE_COMMAND,
            WEBUI_V2_OPERATOR_HANDLER_COMMAND,
            WEBUI_V2_OPERATOR_MOUNT_COMMAND,
            WEBUI_V2_OPERATOR_LLM_CONFIG_COMMAND,
        ],
        notes=(
            "Covers non-browser WebUI v2 operator/LLM configuration rows: "
            "descriptor policy, provider CRUD and active/test/model routes, "
            "LLM provider substrate contracts, "
            "operator setup/config/diagnostics/status/logs/lifecycle handlers, "
            "operator capability/mount gating, redacted secret/error handling, "
            "and composed provider key persistence."
        ),
    ),
    "webui_v2_provider_login_api_regression": CaseSpec(
        name="webui_v2_provider_login_api_regression",
        feature="WebUI v2 NEAR AI and Codex provider login APIs",
        category="Hermetic Provider Login API Regression",
        qa_matrix_test_ids=[
            "REBCLI-097-TC-01",
            "REBCLI-097-TC-02",
            "REBCLI-097-TC-03",
            "REBCLI-097-TC-04",
            "REBCLI-097-TC-05",
            "REBCLI-097-TC-06",
            "REBCLI-097-TC-07",
            "REBCLI-097-TC-08",
            "REBCLI-097-TC-09",
        ],
        commands=[
            REBORN_CLI_WEBUI_V2_BINARY_COMMAND,
            WEBUI_V2_LLM_PROVIDER_ROUTE_COMMAND,
            WEBUI_V2_NEARAI_LOGIN_STATE_COMMAND,
            WEBUI_V2_PROVIDER_LOGIN_MOUNT_COMMAND,
            WEBUI_V2_PROVIDER_LOGIN_BROWSER_COMMAND,
        ],
        notes=(
            "Covers the API/runtime and browser provider-login rows without "
            "duplicating PR #5348 browser settings coverage: route dispatch, operator "
            "authorization, NEAR AI login origin/state/callback policy, Codex "
            "login route protection, wallet route protection, and multi-user "
            "route suppression, plus committed Settings browser coverage for "
            "NEAR AI hosted-login body/origin, Codex device-code UI, and "
            "browser-visible NEAR AI/Codex start-failure errors."
        ),
    ),
}


def parse_duration_seconds(raw: str | None) -> int:
    if raw is None or not raw.strip():
        return DEFAULT_TIMEOUT_SECONDS
    value = raw.strip().lower()
    match = re.fullmatch(r"(\d+)([smh]?)", value)
    if not match:
        raise ValueError(f"invalid duration {raw!r}; use seconds, 30s, 45m, or 1h")
    amount = int(match.group(1))
    unit = match.group(2)
    if unit == "h":
        return amount * 60 * 60
    if unit == "m":
        return amount * 60
    return amount


def render_command(command: CommandSpec) -> str:
    unset_prefix = " ".join(f"unset {shlex.quote(name)};" for name in command.unset_env)
    env_prefix = " ".join(
        f"{name}={shlex.quote(value)}" for name, value in sorted(command.env.items())
    )
    rendered = " ".join(shlex.quote(part) for part in command.argv)
    prefix = " ".join(part for part in [unset_prefix, env_prefix] if part)
    if prefix:
        return f"{prefix} {rendered}"
    return rendered


def _now_iso() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def _safe_log_name(name: str) -> str:
    return re.sub(r"[^A-Za-z0-9_.-]+", "_", name)


def _selected_case_names(args: argparse.Namespace) -> list[str]:
    if not args.case:
        return [name for name, spec in CASES.items() if spec.default_enabled]
    names: list[str] = []
    for name in args.case:
        if name not in CASES:
            raise SystemExit(f"unknown case {name!r}; valid cases: {', '.join(CASES)}")
        if name not in names:
            names.append(name)
    return names


def _test_ids_for(cases: list[CaseSpec]) -> list[str]:
    return sorted({test_id for case in cases for test_id in case.qa_matrix_test_ids})


def write_case_manifest(output_dir: Path, selected_cases: list[str]) -> Path:
    output_dir.mkdir(parents=True, exist_ok=True)
    selected_specs = [CASES[name] for name in selected_cases]
    all_specs = list(CASES.values())
    matrix_path = os.environ.get("REBORN_QA_MATRIX_PATH", "").strip()
    manifest = {
        "generated_at": _now_iso(),
        "selected_cases": selected_cases,
        "default_cases": [
            name for name, spec in CASES.items() if spec.default_enabled
        ],
        "qa_matrix": {
            "source": "local_xlsx",
            "path": matrix_path or None,
            "represented_test_ids": _test_ids_for(all_specs),
            "represented_test_id_count": len(_test_ids_for(all_specs)),
            "selected_represented_test_ids": _test_ids_for(selected_specs),
            "selected_represented_test_id_count": len(_test_ids_for(selected_specs)),
        },
        "cases": [
            {
                "case": name,
                "feature": spec.feature,
                "category": spec.category,
                "qa_matrix_test_ids": spec.qa_matrix_test_ids,
                "default_enabled": spec.default_enabled,
                "mode": MODE,
                "notes": spec.notes,
                "commands": [
                    {
                        "name": command.name,
                        "description": command.description,
                        "command": render_command(command),
                    }
                    for command in spec.commands
                ],
            }
            for name, spec in CASES.items()
        ],
    }
    path = output_dir / "case-manifest.json"
    path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return path


def run_command(
    command: CommandSpec,
    *,
    output_dir: Path,
    case_name: str,
    timeout_seconds: int,
    dry_run: bool,
) -> dict[str, Any]:
    log_base = f"{_safe_log_name(case_name)}.{_safe_log_name(command.name)}"
    stdout_log = output_dir / f"{log_base}.stdout.log"
    stderr_log = output_dir / f"{log_base}.stderr.log"
    details: dict[str, Any] = {
        "name": command.name,
        "description": command.description,
        "command": render_command(command),
        "stdout_log": str(stdout_log),
        "stderr_log": str(stderr_log),
        "dry_run": dry_run,
    }
    if dry_run:
        stdout_log.write_text("", encoding="utf-8")
        stderr_log.write_text("", encoding="utf-8")
        details.update({"success": True, "returncode": None, "latency_ms": 0})
        return details

    env = os.environ.copy()
    for name in command.unset_env:
        env.pop(name, None)
    env.update(command.env)
    started = time.monotonic()
    with stdout_log.open("w", encoding="utf-8") as stdout, stderr_log.open(
        "w", encoding="utf-8"
    ) as stderr:
        try:
            completed = subprocess.run(
                command.argv,
                cwd=ROOT,
                env=env,
                stdout=stdout,
                stderr=stderr,
                text=True,
                timeout=timeout_seconds,
                check=False,
            )
            returncode: int | None = completed.returncode
            success = completed.returncode == 0
            error = None
        except subprocess.TimeoutExpired:
            stderr.write(
                f"\nTimed out after {timeout_seconds} seconds: "
                f"{render_command(command)}\n"
            )
            returncode = None
            success = False
            error = "timeout"
    details.update(
        {
            "success": success,
            "returncode": returncode,
            "latency_ms": int((time.monotonic() - started) * 1000),
        }
    )
    if error:
        details["error"] = error
    return details


def run_case(
    case: CaseSpec,
    *,
    output_dir: Path,
    timeout_seconds: int,
    dry_run: bool,
) -> dict[str, Any]:
    started = time.monotonic()
    command_results: list[dict[str, Any]] = []
    failed = False
    for command in case.commands:
        if failed:
            command_results.append(
                {
                    "name": command.name,
                    "description": command.description,
                    "command": render_command(command),
                    "success": False,
                    "skipped": True,
                    "reason": "previous command failed",
                }
            )
            continue
        result = run_command(
            command,
            output_dir=output_dir,
            case_name=case.name,
            timeout_seconds=timeout_seconds,
            dry_run=dry_run,
        )
        command_results.append(result)
        failed = not bool(result["success"])

    success = all(bool(result.get("success")) for result in command_results)
    return {
        "provider": PROVIDER,
        "mode": MODE,
        "case": case.name,
        "feature": case.feature,
        "category": case.category,
        "success": success,
        "latency_ms": int((time.monotonic() - started) * 1000),
        "details": {
            "qa_matrix_test_ids": case.qa_matrix_test_ids,
            "commands": command_results,
            "notes": case.notes,
        },
    }


def write_results(
    output_dir: Path,
    *,
    selected_cases: list[str],
    timeout_seconds: int,
    dry_run: bool,
    results: list[dict[str, Any]],
) -> Path:
    passed = sum(1 for result in results if result["success"])
    failed = len(results) - passed
    payload = {
        "provider": PROVIDER,
        "mode": MODE,
        "generated_at": _now_iso(),
        "success": failed == 0,
        "dry_run": dry_run,
        "selected_cases": selected_cases,
        "timeout_seconds": timeout_seconds,
        "summary": {
            "passed": passed,
            "failed": failed,
            "total": len(results),
            "qa_matrix_test_ids": _test_ids_for([CASES[name] for name in selected_cases]),
        },
        "results": results,
    }
    path = output_dir / "results.json"
    path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    return path


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=DEFAULT_OUTPUT_DIR,
        help=f"artifact directory (default: {DEFAULT_OUTPUT_DIR})",
    )
    parser.add_argument(
        "--case",
        action="append",
        help="case name to execute; may be repeated; defaults to all default cases",
    )
    parser.add_argument(
        "--timeout",
        default=os.environ.get("COMMAND_TIMEOUT"),
        help="per-command timeout, e.g. 1800, 30m, or 1h",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="write manifest/results without executing cargo commands",
    )
    parser.add_argument(
        "--list-cases",
        action="store_true",
        help="print available cases and exit",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    if args.list_cases:
        for name, spec in CASES.items():
            default = "default" if spec.default_enabled else "targeted"
            print(f"{name}\t{default}\t{','.join(spec.qa_matrix_test_ids)}")
        return 0
    timeout_seconds = parse_duration_seconds(args.timeout)
    selected_cases = _selected_case_names(args)
    args.output_dir.mkdir(parents=True, exist_ok=True)
    write_case_manifest(args.output_dir, selected_cases)
    results = [
        run_case(
            CASES[name],
            output_dir=args.output_dir,
            timeout_seconds=timeout_seconds,
            dry_run=args.dry_run,
        )
        for name in selected_cases
    ]
    results_path = write_results(
        args.output_dir,
        selected_cases=selected_cases,
        timeout_seconds=timeout_seconds,
        dry_run=args.dry_run,
        results=results,
    )
    print(str(results_path))
    return 0 if all(result["success"] for result in results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
