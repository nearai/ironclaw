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

use quote::ToTokens;
use ratchet_support::workspace_root;
use syn::spanned::Spanned;
use syn::{Attribute, Fields, ImplItem, Item};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct FrozenPathCount {
    category: &'static str,
    item_kind: &'static str,
    path: &'static str,
    count: usize,
}

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
        item_kind: "method",
        path: "crates/ironclaw_extension_host/src/channel_pairing.rs",
        count: 1,
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
        path: "crates/ironclaw_host_runtime/src/first_party_tools/memory.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "field",
        path: "crates/ironclaw_reborn_composition/src/factory.rs",
        count: 11,
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
        count: 12,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "field",
        path: "crates/ironclaw_reborn_composition/src/runtime_input.rs",
        count: 3,
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
        path: "crates/ironclaw_event_projections/src/pending_gate_projection.rs",
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
        count: 7,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_host_runtime/src/first_party_tools/memory.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_host_runtime/src/obligations.rs",
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
        path: "crates/ironclaw_process_sandbox/src/docker.rs",
        count: 1,
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
        path: "crates/ironclaw_reborn_composition/src/automation/service.rs",
        count: 1,
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
        path: "crates/ironclaw_reborn_composition/src/observability/trace_capture.rs",
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
        path: "crates/ironclaw_reborn_composition/src/runtime/local_dev.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_reborn_composition/src/runtime/local_dev/extension_surface.rs",
        count: 2,
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
        path: "crates/ironclaw_reborn_composition/src/runtime_input.rs",
        count: 5,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_reborn_composition/src/support/fs/attachment_landing.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_reborn_composition/src/support/fs/project_filesystem_reader.rs",
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
        path: "crates/ironclaw_runner/src/loop_exit_applier.rs",
        count: 7,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_runner/src/tool_disclosure.rs",
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
        path: "crates/ironclaw_host_api/src/product_adapter/auth.rs",
        count: 5,
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
        path: "crates/ironclaw_reborn_composition/src/runtime/local_dev/extension_surface.rs",
        count: 1,
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
        path: "crates/ironclaw_approvals/src/cas_record.rs",
        count: 3,
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
        path: "crates/ironclaw_extension_host/src/channel_identity_store.rs",
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
        path: "crates/ironclaw_host_runtime/src/services/runtime_adapters.rs",
        count: 1,
    },
    FrozenPathCount {
        category: "test-support",
        item_kind: "method",
        path: "crates/ironclaw_reborn_composition/src/runtime/local_dev/extension_surface.rs",
        count: 2,
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
        path: "crates/ironclaw_threads/src/filesystem_service.rs",
        count: 1,
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct Occurrence {
    category: &'static str,
    item_kind: &'static str,
    line: usize,
    member: String,
}

fn attr_categories(attrs: &[Attribute]) -> Vec<&'static str> {
    let joined = attrs
        .iter()
        .map(|attribute| attribute.meta.to_token_stream().to_string())
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    let mut categories = Vec::new();
    if joined.contains("cfg(test)")
        || joined.contains("cfg(any(test")
        || joined.contains("feature = \"test-support\"")
        || joined.contains("feature=\"test-support\"")
    {
        categories.push("test-support");
    }
    if joined.contains("allow(dead_code") || joined.contains("expect(dead_code") {
        categories.push("dead-code");
    }
    categories
}

fn scan_member_occurrences(source: &str) -> Vec<Occurrence> {
    let mut occurrences = Vec::new();
    let file =
        syn::parse_file(source).unwrap_or_else(|err| panic!("failed to parse fixture: {err}"));
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
        if in_test_module || is_test_item && matches!(item, Item::Mod(_)) {
            if let Item::Mod(item) = item
                && let Some((_, nested)) = &item.content
            {
                scan_items(nested, true, occurrences);
            }
            continue;
        }
        match item {
            Item::Struct(item) => {
                if let Fields::Named(fields) = &item.fields {
                    for field in &fields.named {
                        for category in attr_categories(&field.attrs) {
                            occurrences.push(Occurrence {
                                category,
                                item_kind: "field",
                                line: field.span().start().line,
                                member: field
                                    .ident
                                    .as_ref()
                                    .map_or_else(|| "_".to_string(), ToString::to_string),
                            });
                        }
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
        for occurrence in scan_member_occurrences(&contents) {
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

    let frozen: BTreeMap<(String, String, String), usize> = FROZEN_PATH_COUNTS
        .iter()
        .map(|entry| {
            (
                (
                    entry.category.to_string(),
                    entry.item_kind.to_string(),
                    entry.path.to_string(),
                ),
                entry.count,
            )
        })
        .collect();

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
        mod tests {
            struct Fixture {
                #[allow(dead_code)]
                field: usize,
            }
        }
    "#;
    let got: Vec<(String, &'static str, &'static str)> = scan_member_occurrences(source)
        .into_iter()
        .map(|occurrence| (occurrence.member, occurrence.category, occurrence.item_kind))
        .collect();
    assert_eq!(
        got,
        vec![
            ("test_only".to_string(), "test-support", "field"),
            ("reserved".to_string(), "dead-code", "field"),
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
