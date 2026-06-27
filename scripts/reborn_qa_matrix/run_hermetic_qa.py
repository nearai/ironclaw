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

CASES: dict[str, CaseSpec] = {
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
        ],
        commands=[
            WEBUI_V2_DESCRIPTOR_POLICY_COMMAND,
            WEBUI_V2_PROJECT_HANDLER_COMMAND,
            WEBUI_V2_PROJECTS_HANDLER_COMMAND,
            WEBUI_V2_MEMBER_HANDLER_COMMAND,
        ],
        notes=(
            "Covers WebUI v2 project and membership API rows without "
            "duplicating PR #5348 browser project overview coverage: "
            "descriptor policy, project collection and item routes, path/body "
            "ID precedence, member add/update/remove routing, unwired service "
            "fail-closed behavior, no-content delete responses, and "
            "reborn-projects session feature projection."
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
        ],
        commands=[
            WEBUI_V2_SSO_AUTH_ROUTE_COMMAND,
            WEBUI_V2_GOOGLE_OAUTH_ROUTE_COMMAND,
            WEBUI_V2_GITHUB_OAUTH_ROUTE_COMMAND,
            WEBUI_V2_SESSION_ROUND_TRIP_COMMAND,
            WEBUI_V2_SSO_NETWORK_LIMITS_COMMAND,
            WEBUI_V2_SSO_PUBLIC_MOUNT_COMMAND,
        ],
        notes=(
            "Covers WebUI v2 public SSO session rows without live provider "
            "calls: provider discovery, Google/GitHub login redirect and "
            "callback success/failure, one-time state/ticket replay guards, "
            "session bearer use on protected WebUI v2 routes, logout "
            "revocation, public route mount policy, open-redirect defense, "
            "body/rate limits, CORS fail-closed behavior, and sanitized errors."
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
        ],
        commands=[
            WEBUI_V2_PRODUCT_AUTH_OAUTH_COMMAND,
            WEBUI_V2_PRODUCT_AUTH_GOOGLE_OAUTH_COMMAND,
            WEBUI_V2_PRODUCT_AUTH_CALLBACK_COMMAND,
        ],
        notes=(
            "Covers WebUI v2 product-auth OAuth API/runtime rows without "
            "duplicating PR #5348 browser auth-card coverage: generic and "
            "Google OAuth start/callback success, browser completion without "
            "secret leakage, provider denial/exchange failure, malformed and "
            "unknown callback state, invalid scope/expiry rejection, "
            "cross-scope rejection, bearer/no-body enforcement, and "
            "per-caller/per-IP rate limits."
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
        ],
        commands=[
            WEBUI_V2_MANUAL_TOKEN_LEGACY_COMMAND,
            WEBUI_V2_MANUAL_TOKEN_SPLIT_COMMAND,
            WEBUI_V2_MANUAL_TOKEN_FACADE_COMMAND,
        ],
        notes=(
            "Covers WebUI v2 manual-token API/runtime rows without "
            "duplicating PR #5348 browser auth-card coverage: legacy submit "
            "success and redaction, split setup/secret-submit success, seeded "
            "gate projection, invalid secret redaction, abandoned interactions "
            "on submit failure, partial continuation rejection, missing "
            "invocation enforcement, bearer/body/rate-limit enforcement, "
            "facade retry/cross-scope/fail-closed behavior, and sanitized "
            "backend failures."
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
        ],
        commands=[
            WEBUI_V2_DESCRIPTOR_POLICY_COMMAND,
            WEBUI_V2_LLM_PROVIDER_ROUTE_COMMAND,
            WEBUI_V2_OPERATOR_HANDLER_COMMAND,
            WEBUI_V2_OPERATOR_MOUNT_COMMAND,
            WEBUI_V2_OPERATOR_LLM_CONFIG_COMMAND,
        ],
        notes=(
            "Covers non-browser WebUI v2 operator/LLM configuration rows: "
            "descriptor policy, provider CRUD and active/test/model routes, "
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
        ],
        commands=[
            WEBUI_V2_LLM_PROVIDER_ROUTE_COMMAND,
            WEBUI_V2_NEARAI_LOGIN_STATE_COMMAND,
            WEBUI_V2_PROVIDER_LOGIN_MOUNT_COMMAND,
        ],
        notes=(
            "Covers the API/runtime provider-login rows without duplicating "
            "PR #5348 browser settings coverage: route dispatch, operator "
            "authorization, NEAR AI login origin/state/callback policy, Codex "
            "login route protection, wallet route protection, and multi-user "
            "route suppression."
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
    prefix = " ".join(
        f"{name}={shlex.quote(value)}" for name, value in sorted(command.env.items())
    )
    rendered = " ".join(shlex.quote(part) for part in command.argv)
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
