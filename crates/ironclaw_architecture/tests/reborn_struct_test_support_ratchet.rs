//! Static ratchet for test-only and dead-code struct members in production
//! source.
//!
//! Production structs should not grow fields or methods that exist only to
//! support tests, and they should not grow dead-code-suppressed members. The
//! current tree has existing examples (mostly composition/runtime test seams),
//! so this test freezes the per-file inventory by category and member kind. A
//! new occurrence fails by increasing a path count; removing one should shrink
//! the matching baseline entry in the same PR.

#[allow(dead_code)]
mod ratchet_support;

use std::collections::BTreeMap;
use std::path::Path;

use ratchet_support::{try_resolve_crate_relative, workspace_root};
use syn::parse::Parser;
use syn::spanned::Spanned;
use syn::{Attribute, Fields, ImplItem, Item, Meta, Token};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct FrozenPathCount {
    category: &'static str,
    item_kind: &'static str,
    path: &'static str,
    count: usize,
}

/// WS0 baselines for the production-struct dead-code ratchet (target-
/// architecture epic #3773, workstream #6920): the frozen inventory's size
/// measured **on this checkout**, not copied from the design docs.
///
/// Measured 2026-07-30 against `origin/main` @ `ae0989c37` from
/// `FROZEN_PATH_COUNTS` below — 82 frozen `(category, kind, path)` entries
/// totalling 283 suppressed members (both printed by the assertion at the end
/// of the gate below on failure). Lowered to 81/282 by the **WS8** dead-module
/// deletion (#6943), which removed
/// `event_projections/src/pending_gate_projection.rs` and its one
/// test-support method. Lowered again to **80/277** by **WS1.5**, which
/// deleted all five `dead-code` suppressions in
/// `host_api/src/product_adapter/auth.rs`: they existed only because the mint
/// family's constructors were `#[cfg(feature = "host-auth-mint")]`, so a
/// feature-off build saw them as unreachable. Witness-gated minting compiles
/// unconditionally, so the constructors have real callers in every build and
/// the suppressions are not merely moved — they are unnecessary. The file's
/// two `test-support` methods (`test_verified`/`test_verified_for_tenant`)
/// stay and keep their entry. (The `WS0_` prefix records when the baseline was
/// *captured*, by #6936; deletions that lower it come from WS8 and, here,
/// WS1.5.)
///
/// The per-path checks already refuse growth *within* a listed file and refuse
/// staleness. These two totals close the remaining door: adding a **new** path
/// entry to the frozen list — the one way debt can legitimately grow — now has
/// to move a recorded number, so it is a reviewed decision rather than one
/// more line in a long list. Both only ever move down; lower them in the same
/// PR that deletes debt (CHECKLIST WS8 flips `dead_code`/`unreachable_pub` on
/// workspace-wide once this inventory is gone).
///
/// ✎ **80/277 → 79/276 (2026-08-04, #7147), and the totals check below is now
/// an equality.** "Both only ever move down; lower them in the same PR that
/// deletes debt" was honour-system, and it had already been skipped once: a
/// `<=` ratchet is silent when the constant sits *above* the inventory, so the
/// tree carried **one frozen path and one member of untracked slack** — enough
/// for a whole new frozen dead-code path with one suppressed member to be
/// appended and every gate stay green. That is precisely the growth this
/// totals check exists to catch, and it could not see it. Recounted on
/// `origin/main` @ `676d86ce02` by zeroing both constants and reading the
/// panic (which prints `FROZEN_PATH_COUNTS.len()` and the member sum), not by
/// eye: **79 paths / 276 members**.
const WS0_PRODUCTION_STRUCT_DEBT_PATH_BASELINE: usize = 79;
const WS0_PRODUCTION_STRUCT_DEBT_MEMBER_BASELINE: usize = 276;

const FROZEN_PATH_COUNTS: &[FrozenPathCount] = &[
    FrozenPathCount {
        category: "dead-code",
        item_kind: "field",
        path: "crates/ironclaw_hooks/src/self_authored.rs",
        count: 2,
    },
    FrozenPathCount {
        category: "dead-code",
        item_kind: "field",
        path: "crates/ironclaw_llm/src/nearai_chat.rs",
        count: 4,
    },
    FrozenPathCount {
        category: "dead-code",
        item_kind: "field",
        path: "crates/ironclaw_reborn_composition/src/factory.rs",
        count: 2,
    },
    FrozenPathCount {
        category: "dead-code",
        item_kind: "field",
        path: "crates/ironclaw_reborn_composition/src/runtime.rs",
        count: 2,
    },
    FrozenPathCount {
        category: "dead-code",
        item_kind: "method",
        path: "crates/ironclaw_auth/src/product_auth/api/auth.rs",
        count: 3,
    },
    FrozenPathCount {
        category: "dead-code",
        item_kind: "method",
        path: "crates/ironclaw_extension_host/src/channel_pairing.rs",
        count: 1,
    },
    // Unwired sandbox credential-firewall primitives (W5/W8). Retire these
    // four entries when the egress proxy consumer (W6) lands on main and the
    // `#[allow(dead_code)]` attributes come off — the `removed` assertion
    // below only shrinks this baseline if the attributes are actually
    // deleted, not merely left in place.
    FrozenPathCount {
        category: "dead-code",
        item_kind: "method",
        path: "crates/ironclaw_sandbox/src/sandbox_process/ca.rs",
        count: 4,
    },
    // Same W6 retirement trigger as the `ca.rs` entry above.
    FrozenPathCount {
        category: "dead-code",
        item_kind: "method",
        path: "crates/ironclaw_sandbox/src/sandbox_process/credential_firewall.rs",
        count: 5,
    },
    FrozenPathCount {
        category: "dead-code",
        item_kind: "method",
        path: "crates/ironclaw_sandbox/src/sandbox_process/attribution.rs",
        count: 4,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "field",
        path: "crates/ironclaw_extension_host/src/channel_host.rs",
        count: 2,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "field",
        path: "crates/ironclaw_host_runtime/src/services/runtime_adapters.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "field",
        path: "crates/ironclaw_reborn_composition/src/extension_host_assembly.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "field",
        path: "crates/ironclaw_reborn_composition/src/factory.rs",
        count: 9,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "field",
        path: "crates/ironclaw_reborn_composition/src/factory/production_build_assembly.rs",
        count: 2,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "field",
        path: "crates/ironclaw_reborn_composition/src/input.rs",
        count: 2,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "field",
        path: "crates/ironclaw_reborn_composition/src/runtime.rs",
        count: 11,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "field",
        path: "crates/ironclaw_reborn_composition/src/runtime_input.rs",
        count: 3,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "field",
        path: "crates/ironclaw_resources/src/filesystem_governor.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "field",
        path: "crates/ironclaw_resources/src/filesystem_governor/journal.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_agent_loop/src/executor/input.rs",
        count: 2,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_approvals/src/cas_record.rs",
        count: 3,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_auth/src/fakes.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_auth/src/product_auth/api/auth.rs",
        count: 3,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_auth/src/product_auth/credentials/runtime_credentials.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_auth/src/product_auth/durable/mod.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_extension_host/src/available_extensions.rs",
        count: 2,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_extension_host/src/capability_surface.rs",
        count: 2,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_extension_host/src/channel_host.rs",
        count: 2,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_extension_host/src/channel_identity_binding.rs",
        count: 2,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_extension_host/src/channel_identity_store.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_extension_host/src/channel_pairing.rs",
        count: 4,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_hooks/src/dispatch/mod.rs",
        count: 2,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_hooks/src/evaluator.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_hooks/src/points/capability.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_hooks/src/sink.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_host_api/src/authorized.rs",
        count: 2,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_host_api/src/product_adapter/auth.rs",
        count: 2,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_sandbox/src/sandbox_process/ca.rs",
        count: 2,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_sandbox/src/sandbox_process/credential_firewall.rs",
        count: 1,
    },
    // WS3 split `obligations.rs` into one module per chartered owner; the
    // single frozen test-support method travelled to the staged-handoff owner.
    // Repointed, not re-baselined: the count is unchanged at 1.
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_host_runtime/src/obligations/staged_handoffs.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_sandbox/src/sandbox_process/attribution.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_host_runtime/src/services.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_host_runtime/src/services/production_wiring.rs",
        count: 2,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_host_runtime/src/services/runtime_adapters.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_host_runtime/src/user_profile_source.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_host_runtime/src/wasm_credentials.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_llm/src/codex_chatgpt.rs",
        count: 2,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_loop_host/src/cancellation_port.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_loop_host/src/subagent_spawn_port.rs",
        count: 3,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_operator/src/operator_service_lifecycle.rs",
        count: 5,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_product/src/inbound_turn.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_product/src/projection/display_preview.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_product/src/projection/turn_events.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_product/src/run_delivery/triggered.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_reborn_cli/src/context.rs",
        count: 2,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_reborn_composition/src/builtin_capability_policy.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_reborn_composition/src/factory/test_support.rs",
        count: 51,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_reborn_composition/src/input.rs",
        count: 4,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        // Re-keyed 2026-08-04 (WS6) from
        // `ironclaw_reborn_composition/src/observability/trace_capture.rs`: the
        // module moved to the turn-runner observer seam. Same struct, same
        // `#[cfg(test)] with_history_source` seam, same count — a path-keyed
        // inventory entry following its file, which is the WS10 hazard this
        // gate exists to make loud rather than a new allowance.
        path: "crates/ironclaw_runner/src/trace_capture.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_reborn_composition/src/runtime.rs",
        count: 44,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_reborn_composition/src/runtime/capability_host.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_reborn_composition/src/runtime/runtime_turn_scheduler.rs",
        count: 3,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_reborn_composition/src/runtime/test_support.rs",
        count: 4,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_reborn_composition/src/runtime_input.rs",
        count: 5,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_product/src/scoped_fs/attachment_reader.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_reborn_traces/src/onboarding/device_key.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_resources/src/filesystem_governor.rs",
        count: 3,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_resources/src/filesystem_governor/journal.rs",
        count: 3,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_runner/src/loop_exit_applier.rs",
        count: 7,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_loop_host/src/tool_disclosure.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_threads/src/filesystem_service.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_threads/src/in_memory.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_triggers/src/worker/ports.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_trust/src/sources.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_turns/src/loop_exit.rs",
        count: 14,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_webui/src/auth/github.rs",
        count: 2,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_webui/src/auth/google.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_webui/src/product_auth/mod.rs",
        count: 2,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_webui/src/webui_v2/sse_capacity.rs",
        count: 2,
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct Occurrence {
    category: &'static str,
    item_kind: &'static str,
    line: usize,
    member: String,
}

fn nested_meta(meta: &Meta) -> Option<Vec<Meta>> {
    let Meta::List(list) = meta else {
        return None;
    };
    syn::punctuated::Punctuated::<Meta, Token![,]>::parse_terminated
        .parse2(list.tokens.clone())
        .ok()
        .map(|metas| metas.into_iter().collect())
}

fn is_test_cfg(meta: &Meta) -> bool {
    match meta {
        Meta::Path(path) => path.is_ident("test"),
        Meta::NameValue(value) => {
            value.path.is_ident("feature")
                && matches!(
                    &value.value,
                    syn::Expr::Lit(expr)
                        if matches!(&expr.lit, syn::Lit::Str(lit) if lit.value() == "test-support")
                )
        }
        Meta::List(list) if list.path.is_ident("any") || list.path.is_ident("all") => {
            nested_meta(meta).is_some_and(|metas| metas.iter().any(is_test_cfg))
        }
        _ => false,
    }
}

fn is_dead_code_lint(meta: &Meta) -> bool {
    matches!(
        meta,
        Meta::Path(path) if path.is_ident("dead_code")
    )
}

fn is_lint_attribute(meta: &Meta) -> bool {
    let Meta::List(list) = meta else {
        return false;
    };
    (list.path.is_ident("allow") || list.path.is_ident("expect"))
        && nested_meta(meta).is_some_and(|metas| metas.iter().any(is_dead_code_lint))
}

fn attr_categories(attrs: &[Attribute]) -> Vec<&'static str> {
    let mut categories = Vec::new();
    let mut test_support = false;
    let mut dead_code = false;
    for attribute in attrs {
        match &attribute.meta {
            Meta::List(list) if list.path.is_ident("cfg") => {
                test_support |=
                    nested_meta(&attribute.meta).is_some_and(|metas| metas.iter().any(is_test_cfg));
            }
            Meta::List(list) if list.path.is_ident("cfg_attr") => {
                if let Some(metas) = nested_meta(&attribute.meta) {
                    test_support |= metas.first().is_some_and(is_test_cfg);
                    dead_code |= metas.iter().skip(1).any(is_lint_attribute);
                }
            }
            meta => dead_code |= is_lint_attribute(meta),
        }
    }
    if test_support {
        categories.push("test-support");
    }
    if dead_code {
        categories.push("dead-code");
    }
    categories
}

fn scan_member_occurrences(source: &str, origin: &str) -> Vec<Occurrence> {
    let mut occurrences = Vec::new();
    let file =
        syn::parse_file(source).unwrap_or_else(|err| panic!("failed to parse {origin}: {err}"));
    scan_items(&file.items, false, &mut occurrences);
    occurrences
}

fn scan_items(items: &[Item], in_test_module: bool, occurrences: &mut Vec<Occurrence>) {
    for item in items {
        let attrs = match item {
            Item::Const(item) => &item.attrs,
            Item::Enum(item) => &item.attrs,
            Item::ExternCrate(item) => &item.attrs,
            Item::Fn(item) => &item.attrs,
            Item::ForeignMod(item) => &item.attrs,
            Item::Impl(item) => &item.attrs,
            Item::Macro(item) => &item.attrs,
            Item::Mod(item) => &item.attrs,
            Item::Static(item) => &item.attrs,
            Item::Struct(item) => &item.attrs,
            Item::Trait(item) => &item.attrs,
            Item::TraitAlias(item) => &item.attrs,
            Item::Type(item) => &item.attrs,
            Item::Union(item) => &item.attrs,
            Item::Use(item) => &item.attrs,
            Item::Verbatim(_) => continue,
            _ => continue,
        };
        let is_test_item = attr_categories(attrs).contains(&"test-support");
        if in_test_module || is_test_item {
            if let Item::Mod(item) = item
                && let Some((_, nested)) = &item.content
            {
                scan_items(nested, true, occurrences);
            }
            continue;
        }
        match item {
            Item::Struct(item) => {
                let fields = match &item.fields {
                    Fields::Named(fields) => fields.named.iter().enumerate().collect(),
                    Fields::Unnamed(fields) => fields.unnamed.iter().enumerate().collect(),
                    Fields::Unit => Vec::new(),
                };
                for (index, field) in fields {
                    for category in attr_categories(&field.attrs) {
                        occurrences.push(Occurrence {
                            category,
                            item_kind: "field",
                            line: field.span().start().line,
                            member: field
                                .ident
                                .as_ref()
                                .map_or_else(|| index.to_string(), ToString::to_string),
                        });
                    }
                }
            }
            Item::Impl(item) => {
                for impl_item in &item.items {
                    if let ImplItem::Fn(method) = impl_item {
                        for category in attr_categories(&method.attrs) {
                            occurrences.push(Occurrence {
                                category,
                                item_kind: "method",
                                line: method.sig.ident.span().start().line,
                                member: method.sig.ident.to_string(),
                            });
                        }
                    }
                }
            }
            Item::Mod(item) => {
                if let Some((_, nested)) = &item.content {
                    scan_items(nested, false, occurrences);
                }
            }
            _ => {}
        }
    }
}

fn should_skip(path: &Path) -> bool {
    let display = path.to_string_lossy().replace('\\', "/");
    display.contains("/target/")
        || display.contains("/tests/")
        || display.contains("/examples/")
        || display.contains("/benches/")
        || display.ends_with("/tests.rs")
        || display.ends_with("_tests.rs")
}

fn scan_dir(root: &Path, dir: &Path, out: &mut BTreeMap<(String, String, String), usize>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("failed to read directory entry: {err}"));
        let path = entry.path();
        if path.is_dir() {
            if !should_skip(&path) {
                scan_dir(root, &path, out);
            }
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") || should_skip(&path) {
            continue;
        }

        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        for occurrence in scan_member_occurrences(&contents, &relative) {
            *out.entry((
                occurrence.category.to_string(),
                occurrence.item_kind.to_string(),
                relative.clone(),
            ))
            .or_default() += 1;
        }
    }
}

fn format_count(key: &(String, String, String), count: usize) -> String {
    format!("{} {} in {}: {}", key.0, key.1, key.2, count)
}

#[test]
fn reborn_production_struct_test_support_and_dead_code_members_do_not_grow() {
    let root = workspace_root();
    let mut found = BTreeMap::new();
    scan_dir(&root, &root.join("crates"), &mut found);

    // Frozen paths are compared against paths DISCOVERED on disk, so each is
    // resolved through the crate inventory first: after the family move
    // (PROPOSAL section 5) every entry naming a moved crate would otherwise
    // read as BOTH "new occurrence" and "debt that no longer exists" in the
    // same run - 79 of them (CHECKLIST WS10). A path naming a crate that is
    // really gone keeps its literal and is still reported as removed debt.
    let mut frozen = BTreeMap::new();
    for entry in FROZEN_PATH_COUNTS {
        let key = (
            entry.category.to_string(),
            entry.item_kind.to_string(),
            try_resolve_crate_relative(&root, entry.path)
                .unwrap_or_else(|_| entry.path.to_string()),
        );
        assert!(
            frozen.insert(key.clone(), entry.count).is_none(),
            "duplicate FROZEN_PATH_COUNTS entry: {key:?}"
        );
    }

    let added: Vec<String> = found
        .iter()
        .filter_map(|(key, count)| {
            let frozen_count = frozen.get(key).copied().unwrap_or_default();
            (*count > frozen_count).then(|| format_count(key, *count - frozen_count))
        })
        .collect();
    assert!(
        added.is_empty(),
        "Production struct code must not grow test-support or dead-code fields/methods. \
         Put test seams in test modules/support crates, or remove the unused production \
         member instead of suppressing it. New occurrences:\n{}",
        added.join("\n")
    );

    let removed: Vec<String> = frozen
        .iter()
        .filter_map(|(key, count)| {
            let found_count = found.get(key).copied().unwrap_or_default();
            (found_count < *count).then(|| format_count(key, *count - found_count))
        })
        .collect();
    assert!(
        removed.is_empty(),
        "FROZEN_PATH_COUNTS contains production struct test-support/dead-code debt that \
         no longer exists. Shrink the matching baseline entries in this test:\n{}",
        removed.join("\n")
    );

    // WS0 totals ratchet: the per-path checks above cannot see a *new* path
    // entry being appended to the frozen list, which is the one way this debt
    // can grow without any single path count rising.
    let frozen_members: usize = FROZEN_PATH_COUNTS.iter().map(|entry| entry.count).sum();
    assert!(
        !FROZEN_PATH_COUNTS.is_empty() && frozen_members > 0,
        "FROZEN_PATH_COUNTS is empty ({} paths / {frozen_members} members) — the totals ratchet \
         below would pass having measured nothing. Either the inventory really reached zero \
         (delete this check and both baselines together, and flip the workspace lints per \
         CHECKLIST WS8) or the const was truncated.",
        FROZEN_PATH_COUNTS.len()
    );
    assert!(
        FROZEN_PATH_COUNTS.len() <= WS0_PRODUCTION_STRUCT_DEBT_PATH_BASELINE
            && frozen_members <= WS0_PRODUCTION_STRUCT_DEBT_MEMBER_BASELINE,
        "production-struct debt inventory grew to {} paths / {} members (WS0 baseline {} / {}): \
         this list is shrink-only. Put the seam in a test module or delete the unused member \
         instead of freezing a new path; if the owner has approved new debt, raise the matching \
         WS0_PRODUCTION_STRUCT_DEBT_* baseline in the same PR with the rationale in the PR body.",
        FROZEN_PATH_COUNTS.len(),
        frozen_members,
        WS0_PRODUCTION_STRUCT_DEBT_PATH_BASELINE,
        WS0_PRODUCTION_STRUCT_DEBT_MEMBER_BASELINE
    );
    // The slack half (#7147). A ceiling ABOVE the live inventory is an
    // unclaimed budget for exactly the growth the assertion above refuses:
    // with 80/277 recorded against 79/276 live, one new frozen path carrying
    // one suppressed member landed green. Both totals must therefore *equal*
    // the inventory, so a deletion that forgets to lower the constant is red
    // rather than silently banked as headroom.
    assert!(
        FROZEN_PATH_COUNTS.len() >= WS0_PRODUCTION_STRUCT_DEBT_PATH_BASELINE
            && frozen_members >= WS0_PRODUCTION_STRUCT_DEBT_MEMBER_BASELINE,
        "production-struct debt inventory holds {} paths / {} members but the WS0 baselines are \
         {} / {} — UNTRACKED SLACK of {} paths / {} members. That headroom is a free budget for \
         new frozen debt: a path entry can be appended, or a frozen count raised, and every gate \
         stays green. Lower WS0_PRODUCTION_STRUCT_DEBT_PATH_BASELINE to {} and \
         WS0_PRODUCTION_STRUCT_DEBT_MEMBER_BASELINE to {} in the PR that deleted the debt, which \
         is what the doc comment on them already required (#7147).",
        FROZEN_PATH_COUNTS.len(),
        frozen_members,
        WS0_PRODUCTION_STRUCT_DEBT_PATH_BASELINE,
        WS0_PRODUCTION_STRUCT_DEBT_MEMBER_BASELINE,
        WS0_PRODUCTION_STRUCT_DEBT_PATH_BASELINE.saturating_sub(FROZEN_PATH_COUNTS.len()),
        WS0_PRODUCTION_STRUCT_DEBT_MEMBER_BASELINE.saturating_sub(frozen_members),
        FROZEN_PATH_COUNTS.len(),
        frozen_members
    );
}

#[test]
fn reborn_struct_member_scanner_self_test() {
    let source = r#"
        struct Production {
            live: String,
            #[cfg(test)]
            test_only: usize,
            #[allow(dead_code)]
            reserved: String,
            #[allow(dead_code_extra)]
            near_match: String,
        }

        struct TupleProduction(
            usize,
            #[allow(dead_code)] usize,
        );

        #[cfg(test)]
        struct TestStruct {
            #[allow(dead_code)]
            field: usize,
        }

        #[cfg(test)]
        impl TestStruct {
            #[allow(dead_code)]
            fn method(&self) {}
        }

        impl Production {
            pub fn live(&self) {}

            #[cfg_attr(
                any(test, feature = "test-support"),
                allow(dead_code, reason = "fixture")
            )]
            pub(crate) fn with_fake_state(self) -> Self { self }

        }

        impl Production
            where
                String: Send,
        {
            #[cfg(test)]
            fn multiline_impl_test_support(&self) {}
        }

        impl Production {
            #[expect(dead_code, reason = "fixture")]
            fn reserved_accessor(&self) {}
        }

        #[cfg(test)]
        #[cfg(any(testing))]
        struct NearMatch {
            #[allow(dead_code_extra)]
            field: usize,
        }
    "#;
    let got: Vec<(String, &'static str, &'static str)> =
        scan_member_occurrences(source, "self-test fixture")
            .into_iter()
            .map(|occurrence| (occurrence.member, occurrence.category, occurrence.item_kind))
            .collect();
    assert_eq!(
        got,
        vec![
            ("test_only".to_string(), "test-support", "field"),
            ("reserved".to_string(), "dead-code", "field"),
            ("1".to_string(), "dead-code", "field"),
            ("with_fake_state".to_string(), "test-support", "method"),
            ("with_fake_state".to_string(), "dead-code", "method"),
            (
                "multiline_impl_test_support".to_string(),
                "test-support",
                "method"
            ),
            ("reserved_accessor".to_string(), "dead-code", "method"),
        ],
        "scanner should only count attributes attached to production struct fields \
         and impl methods"
    );
}
