//! LLM-vendor census over the contracts family — the pin §12.11 D-E promised
//! and never got (issue #7150; CHECKLIST WS10).
//!
//! # Why the existing scanner cannot do this
//!
//! `reborn_extension_specificity.rs` derives its forbidden vocabulary from
//! **extension package manifests**. `nearai` is then removed globally by its
//! `TERM_COLLISIONS` (it is also the assistant's own LLM backend id), and
//! `codex`, `openai`, `anthropic`, `claude`, `gpt` are not derived terms in any
//! manifest at all. So `start_nearai_login`, `CodexLoginStart` and their
//! siblings are **structurally invisible** to it — D-E says exactly this. Its
//! three `product_contracts` allowlist entries cover `github`/`google`, the
//! NEAR AI SSO providers, and nothing else.
//!
//! # What D-E ruled, and what it owed
//!
//! §12.11 D-E amended §8.2's vendor rule to sanction *LLM-vendor
//! administration vocabulary* in `ironclaw_product_contracts::operator_llm` —
//! **"that module and nowhere else in the contracts family"** — bounded three
//! ways: no seventh vendor name joins the six DTOs; a *fourth* provider login
//! must arrive as a package or behind a shape that adds no vendor-named method
//! or DTO; and *"a targeted vendor-name census over `operator_llm.rs` is owed
//! with the amendment — otherwise the bound is review discipline rather than
//! enforcement."* That census existed on no ref. This file is it, widened to
//! the whole contracts family because "nowhere else in the contracts family" is
//! a claim about the family, and a census scoped to one file cannot check it.
//!
//! # ⚠ What the census found: D-E's "nowhere else" is not true today
//!
//! Running it turns up a **second** LLM-vendor surface in the contracts family
//! that D-E did not know about: `ironclaw_common::llm_costs`, a per-model price
//! table naming **9 distinct vendors across 91 occurrences** (`claude`, `gpt`,
//! `sonnet`, `opus`, `haiku`, `codex`, `mistral`, `deepseek`, `llama`). It is
//! invisible to the specificity scanner for the same reason `operator_llm` is,
//! which is precisely why nobody had seen it. This gate does not delete it —
//! that is a product decision, not a gate's — but it does the thing the
//! honour-system could not: names it, freezes it, and refuses to let it grow.
//!
//! Two further matches are not vendor coupling at all, and each is pinned by
//! the narrower of the two instruments rather than waved through.
//! `prompt_envelope` carries the identity-override string `"you are chatgpt"`,
//! which is a **safety denylist** — the term must stay there or the detector
//! weakens — so it is a censused scope with a stated basis. `attachment_format`
//! matches `opus` as the **Opus audio codec** (`audio/opus`, `.opus`), a pure
//! term collision with no LLM anywhere in the file, so it is a
//! [`TERM_COLLISION_CARVE_OUTS`] entry instead: the term is neutralised at that
//! path rather than budgeted, and the carve-out itself fails the day it stops
//! matching.
//!
//! # Shape
//!
//! Every scope carries a frozen occurrence count checked as an **equality**, so
//! the census fails on growth *and* on slack (#7147: a ceiling above the live
//! count is an unclaimed budget for exactly the growth it refuses). A vendor
//! hit in any un-censused file in the family fails outright.
//!
//! ⚠ **Every test function here must keep its `reborn_` prefix.** The file name
//! is not what selects it: `code_style.yml` runs
//! `cargo test -p ironclaw_architecture_tests reborn`, and that argument is a **test
//! name** filter, not a path filter. Written without the prefix these gates
//! compiled, passed locally, and reported `running 0 tests` under the exact
//! command CI uses.

#[allow(dead_code)]
mod ratchet_support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

// CENSUS / carve-out paths keep their readable flat `crates/<crate>/…`
// spelling and are RESOLVED through the crate inventory before they are
// compared against a scanned file or opened. Without that, the family move
// (`crates/<family>/ironclaw_*`, PROPOSAL §5) turns every row into a term that
// matches nothing — and a census whose rows match nothing reports a clean
// family it never checked (CHECKLIST WS10).
use ratchet_support::{crate_path, resolve_crate_relative, strip_cfg_test_blocks, workspace_root};

/// LLM-vendor vocabulary, lower-case. Deliberately a **literal list**, not
/// derived from manifests: the whole finding behind D-E is that the
/// manifest-derived vocabulary cannot see these names. Terms are matched with
/// identifier boundaries (see [`vendor_hits`]), so `llama` does not fire inside
/// `ollama` and `opus` does not fire inside `opusculum`.
///
/// `together`, `mistral`-as-a-word and similar English collisions are handled
/// by omission or by [`TERM_COLLISION_CARVE_OUTS`] rather than by loosening the
/// matcher — a matcher that under-matches to avoid noise is the fail-open
/// direction for a census.
const LLM_VENDOR_TERMS: &[&str] = &[
    "anthropic",
    "azure",
    "bedrock",
    "chatgpt",
    "claude",
    "codex",
    "cohere",
    "copilot",
    "deepseek",
    "fireworks",
    "gemini",
    "gpt",
    "groq",
    "grok",
    "haiku",
    "huggingface",
    "jurassic",
    "llama",
    "mistral",
    "nearai",
    "ollama",
    "openai",
    "open_ai",
    "opus",
    "palm",
    "perplexity",
    "sonnet",
    "tinfoil",
    "titan",
    "vertex",
    "xai",
];

/// Path-scoped carve-outs for terms that are not vendor references at all.
/// Narrow by construction: a carve-out that stops matching is a hard failure,
/// so it cannot outlive the collision it describes.
const TERM_COLLISION_CARVE_OUTS: &[(&str, &str, &str)] = &[(
    "crates/ironclaw_common/src/attachment_format.rs",
    "opus",
    "the Opus AUDIO CODEC (`audio/opus`, `.opus`), not the Claude model tier — \
     this file is a MIME/extension alias table and names no LLM",
)];

/// One censused scope: a production file in the contracts family permitted to
/// name LLM vendors, with its frozen occurrence count.
struct CensusScope {
    path: &'static str,
    /// Vendor-term occurrences in production code + string literals (comments
    /// and `#[cfg(test)]` items excluded). Checked as an equality.
    occurrences: usize,
    /// Distinct vendor terms present. Checked as an equality.
    distinct_vendors: usize,
    /// The rule that sanctions this scope, so a reader can audit the sanction
    /// rather than trusting the list.
    basis: &'static str,
}

/// Measured on `origin/main` @ `676d86ce02` (2026-08-04) by this gate's own
/// scanner — the counts are whatever it reports, so the baseline and the
/// measurement can never disagree about method.
const CENSUS: &[CensusScope] = &[
    CensusScope {
        path: "crates/ironclaw_product_contracts/src/operator_llm.rs",
        occurrences: 16,
        distinct_vendors: 2,
        basis: "PROPOSAL §12.11 D-E / §8.2 amendment — LLM-vendor administration \
                vocabulary, this module and nowhere else in the contracts family",
    },
    CensusScope {
        path: "crates/ironclaw_common/src/llm_costs.rs",
        occurrences: 91,
        distinct_vendors: 9,
        basis: "UNSANCTIONED RESIDUE, found by this census (#7150): a per-model price \
                table in the contracts family that D-E's 'nowhere else' does not cover \
                and the specificity scanner cannot see. Frozen and shrink-only pending \
                an owner decision — the model-cost table is a candidate to move beside \
                the llm providers, which §8.2 already sanctions for vendor names",
    },
    CensusScope {
        path: "crates/ironclaw_prompt_envelope/src/lib.rs",
        occurrences: 1,
        distinct_vendors: 1,
        basis: "vendor-safety DENYLIST — the identity-override pattern \"you are chatgpt\"; \
                removing the term weakens the detector, so this is the inverse of vendor \
                coupling (same reasoning as the trace-redaction classifier carve-out)",
    },
];

/// D-E's three structural bounds on `operator_llm`, as numbers.
///
/// "no seventh vendor name joins the six" — six vendor-named DTOs.
/// "a *fourth* provider login must arrive as a package or behind a shape that
/// adds no vendor-named method or DTO" — three vendor-named methods.
/// Distinct vendors is the strictest reading of "no seventh vendor name" and is
/// pinned too, so no reading of the ruling is left unenforced.
const D_E_VENDOR_DTO_CEILING: usize = 6;
const D_E_VENDOR_METHOD_CEILING: usize = 3;
const D_E_DISTINCT_VENDOR_CEILING: usize = 2;

/// The exact sanctioned surface, so a *rename* cannot swap one vendor for
/// another while the counts stay put.
const SANCTIONED_VENDOR_API: &[(&str, &str, &str)] = &[
    ("dto", "NearAiAuthProvider", "nearai"),
    ("dto", "NearAiLoginRequest", "nearai"),
    ("dto", "NearAiLoginStart", "nearai"),
    ("dto", "NearAiWalletLoginRequest", "nearai"),
    ("dto", "NearAiWalletLoginResult", "nearai"),
    ("dto", "CodexLoginStart", "codex"),
    ("method", "start_nearai_login", "nearai"),
    ("method", "complete_nearai_wallet_login", "nearai"),
    ("method", "start_codex_login", "codex"),
];

const SANCTIONED_MODULE: &str = "crates/ironclaw_product_contracts/src/operator_llm.rs";

/// Floor for the family walk. The live tree has six contracts crates and ~90
/// production files; this is far below both, so it catches a broken walk
/// without tripping on ordinary churn.
const MIN_SCANNED_FILES: usize = 40;

// ---------------------------------------------------------------------------
// Scanning
// ---------------------------------------------------------------------------
//
// The strippers below are LOCAL rather than added to `ratchet_support`
// deliberately. `strip_comments_and_strings` there blanks string *contents*,
// which a vendor census must not do — a provider id hides in a string literal,
// not in an identifier. Refactoring the shared lexer to keep strings would put
// a behaviour change under thirty other ratchets to serve one caller, and a
// fail-open regression in a shared stripper is exactly the silent-weakening
// this program keeps finding. These two have their own fixtures below.

/// Blank line and (nested) block comments; **keep** string literal contents.
fn strip_comments(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            let mut depth = 1usize;
            i += 2;
            while i < chars.len() && depth > 0 {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                } else {
                    if chars[i] == '\n' {
                        out.push('\n');
                    }
                    i += 1;
                }
            }
            continue;
        }
        if c == '"' {
            out.push(c);
            i += 1;
            while i < chars.len() {
                if chars[i] == '\\' {
                    out.push(chars[i]);
                    if let Some(next) = chars.get(i + 1) {
                        out.push(*next);
                    }
                    i += 2;
                    continue;
                }
                out.push(chars[i]);
                if chars[i] == '"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Byte offsets at which `term` occurs in `text` with identifier boundaries.
///
/// `_` is a **word separator**, not an identifier-internal character, so
/// `start_nearai_login` matches `nearai`. Getting that wrong is the fail-open
/// direction and it is pinned in the self-test: without it the three D-E
/// methods are invisible and the census reports a surface of six instead of
/// nine.
fn vendor_hits(text: &str, term: &str) -> Vec<usize> {
    let lower = text.to_ascii_lowercase();
    let bytes = text.as_bytes();
    let mut hits = Vec::new();
    let mut search = 0usize;
    while let Some(offset) = lower[search..].find(term) {
        let at = search + offset;
        search = at + 1;
        let before_ok = at == 0 || {
            let previous = bytes[at - 1];
            !previous.is_ascii_alphanumeric()
                || ((previous.is_ascii_lowercase() || previous.is_ascii_digit())
                    && bytes[at].is_ascii_uppercase())
        };
        let end = at + term.len();
        let after_ok = end >= bytes.len() || {
            let next = bytes[end];
            !next.is_ascii_alphanumeric() || next.is_ascii_uppercase()
        };
        if before_ok && after_ok {
            hits.push(at);
        }
    }
    hits
}

fn cargo_metadata() -> Value {
    let manifest_path = workspace_root().join("Cargo.toml");
    let output = Command::new("cargo")
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
        ])
        .arg(&manifest_path)
        .output()
        .unwrap_or_else(|error| panic!("failed to run cargo metadata: {error}"));
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("cargo metadata output must be JSON")
}

/// `src` roots of every crate declaring `layer = "contracts"`, resolved through
/// `cargo metadata`'s `manifest_path` so the WS7 family move cannot take this
/// gate dark by relocating a directory.
fn contracts_family_src_roots(metadata: &Value) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for package in metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages")
    {
        let Some(name) = package["name"].as_str() else {
            continue;
        };
        if !(name == "ironclaw" || name.starts_with("ironclaw_")) {
            continue;
        }
        let layer = package
            .get("metadata")
            .and_then(|m| m.get("ironclaw"))
            .and_then(|i| i.get("layer"))
            .and_then(Value::as_str);
        if layer != Some("contracts") {
            continue;
        }
        let manifest = package["manifest_path"]
            .as_str()
            .unwrap_or_else(|| panic!("{name} has no manifest_path"));
        let src = Path::new(manifest)
            .parent()
            .unwrap_or_else(|| panic!("{manifest} has no parent"))
            .join("src");
        assert!(
            src.is_dir(),
            "{name} declares layer=contracts but {} does not exist — a census that walks a \
             missing tree scans nothing and passes",
            src.display()
        );
        roots.push(src);
    }
    roots
}

fn is_test_path(relative: &str) -> bool {
    let name = relative.rsplit('/').next().unwrap_or(relative);
    relative.contains("/tests/")
        || relative.contains("/test_support/")
        || name == "tests.rs"
        || name == "test.rs"
        || name == "test_support.rs"
        || name.ends_with("_tests.rs")
        || name.ends_with("_test.rs")
        || name.starts_with("test_")
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("read_dir {}: {error}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| panic!("entry under {}: {error}", dir.display()));
        let path = entry.path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// `relative path -> (term -> occurrences)` over production code + string
/// literals in the contracts family, plus the number of files scanned.
fn scan_contracts_family() -> (BTreeMap<String, BTreeMap<String, usize>>, usize) {
    let root = workspace_root();
    let metadata = cargo_metadata();
    let mut found: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    let mut scanned = 0usize;

    // Resolved ONCE: `resolve_crate_relative` walks the whole crate tree, so
    // doing it per file per term turns the scan quadratic.
    let carve_outs: Vec<(String, &str)> = TERM_COLLISION_CARVE_OUTS
        .iter()
        .map(|(path, term, _)| (resolve_crate_relative(&root, path), *term))
        .collect();

    for src in contracts_family_src_roots(&metadata) {
        let mut files = Vec::new();
        collect_rs(&src, &mut files);
        for file in files {
            let relative = file
                .strip_prefix(&root)
                .unwrap_or(&file)
                .to_string_lossy()
                .replace('\\', "/");
            if is_test_path(&relative) {
                continue;
            }
            scanned += 1;
            let raw = std::fs::read_to_string(&file)
                .unwrap_or_else(|error| panic!("read {}: {error}", file.display()));
            let text = strip_cfg_test_blocks(&strip_comments(&raw));
            let mut per_term = BTreeMap::new();
            for term in LLM_VENDOR_TERMS {
                let carved = carve_outs
                    .iter()
                    .any(|(path, carved_term)| *path == relative && carved_term == term);
                if carved {
                    continue;
                }
                let count = vendor_hits(&text, term).len();
                if count > 0 {
                    per_term.insert((*term).to_string(), count);
                }
            }
            if !per_term.is_empty() {
                found.insert(relative, per_term);
            }
        }
    }
    (found, scanned)
}

// ---------------------------------------------------------------------------
// Gates
// ---------------------------------------------------------------------------

/// The census itself: the contracts family names LLM vendors only where the
/// census says, in exactly the volume it records.
#[test]
fn reborn_contracts_family_names_llm_vendors_only_in_censused_scopes() {
    assert!(
        !LLM_VENDOR_TERMS.is_empty(),
        "LLM_VENDOR_TERMS is empty — every scan would report zero hits and the census would \
         pass having looked for nothing"
    );
    let (found, scanned) = scan_contracts_family();
    assert!(
        scanned >= MIN_SCANNED_FILES,
        "only {scanned} production files scanned across the contracts family (floor \
         {MIN_SCANNED_FILES}). A census that walks a truncated tree reports a clean family it \
         never read — repoint it rather than letting it measure nothing."
    );
    assert!(
        !found.is_empty(),
        "no vendor terms found anywhere in the contracts family. That would be the target \
         state, but reaching it retires this gate and its CENSUS together — an empty result is \
         far likelier to be a broken stripper, so it fails rather than passes."
    );

    let root = workspace_root();
    // Keyed by the RESOLVED path, because `found` is keyed by where the file
    // really sits; the rows keep their readable flat spelling for messages.
    let censused: BTreeMap<String, &CensusScope> = CENSUS
        .iter()
        .map(|scope| (resolve_crate_relative(&root, scope.path), scope))
        .collect();
    assert_eq!(
        censused.len(),
        CENSUS.len(),
        "CENSUS holds duplicate path rows"
    );
    // Every scope states the rule that sanctions it. A row whose basis is a
    // shrug is an allowlist entry wearing a census row's clothes.
    let unjustified: Vec<&str> = CENSUS
        .iter()
        .filter(|scope| scope.basis.trim().len() < 40)
        .map(|scope| scope.path)
        .collect();
    assert!(
        unjustified.is_empty(),
        "CENSUS rows must state the rule that sanctions them (D-E, a safety denylist, or a \
         named residue with its owner decision): {unjustified:?}"
    );

    // 1. No vendor name outside a censused scope.
    let uncensused: Vec<String> = found
        .iter()
        .filter(|(path, _)| !censused.contains_key(*path))
        .map(|(path, terms)| {
            format!(
                "    {path}: {}",
                terms
                    .iter()
                    .map(|(term, count)| format!("{term}×{count}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .collect();
    assert!(
        uncensused.is_empty(),
        "LLM-VENDOR NAME IN AN UN-CENSUSED CONTRACTS-FAMILY FILE (#7150). §12.11 D-E sanctions \
         this vocabulary in `product_contracts::operator_llm` and NOWHERE ELSE in the contracts \
         family; `reborn_extension_specificity.rs` cannot see these terms at all, which is why \
         this census exists. Move the vendor knowledge to a package or an `llm` provider (both \
         sanctioned by §8.2), or — if the owner amends the ruling — add a CENSUS row stating \
         the basis:\n{}",
        uncensused.join("\n")
    );

    // 2. Every censused scope still names vendors (no stale rows).
    let stale: Vec<&str> = CENSUS
        .iter()
        .filter(|scope| !found.contains_key(&resolve_crate_relative(&root, scope.path)))
        .map(|scope| scope.path)
        .collect();
    assert!(
        stale.is_empty(),
        "CENSUS rows whose file no longer names any vendor (or no longer exists): {stale:?}. \
         Delete the rows so the census keeps shrinking — a scope that describes nothing \
         constrains nothing."
    );

    // 3. Frozen counts, as equalities.
    let mut drift = Vec::new();
    for scope in CENSUS {
        let Some(terms) = found.get(&resolve_crate_relative(&root, scope.path)) else {
            continue;
        };
        let occurrences: usize = terms.values().sum();
        let distinct = terms.len();
        if occurrences != scope.occurrences {
            drift.push(format!(
                "    {}: {occurrences} occurrences, census records {} ({})",
                scope.path,
                scope.occurrences,
                if occurrences > scope.occurrences {
                    "GROWTH — new vendor coupling in a sanctioned scope"
                } else {
                    "SLACK — the scope shrank; lower the census in the same PR so the \
                     improvement is banked rather than left as budget for the next one"
                }
            ));
        }
        if distinct != scope.distinct_vendors {
            drift.push(format!(
                "    {}: {distinct} distinct vendors, census records {} — {:?}",
                scope.path,
                scope.distinct_vendors,
                terms.keys().collect::<Vec<_>>()
            ));
        }
    }
    assert!(
        drift.is_empty(),
        "contracts-family vendor census drift (#7150). Every scope is frozen as an equality, so \
         both directions fail: growth is new coupling, slack is an unclaimed budget for it:\n{}",
        drift.join("\n")
    );
}

/// D-E's three structural bounds on the sanctioned module, enforced as numbers
/// and as an exact roster.
#[test]
fn reborn_d_e_sanctioned_vendor_api_surface_is_frozen() {
    let root = workspace_root();
    let path = crate_path(&root, SANCTIONED_MODULE);
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "{SANCTIONED_MODULE} must be readable — §12.11 D-E's sanction is scoped to this \
             exact module, and a gate that cannot find it enforces nothing: {error}"
        )
    });
    let text = strip_cfg_test_blocks(&strip_comments(&raw));
    assert!(
        text.len() > 1_000,
        "{SANCTIONED_MODULE} stripped to {} chars — the strippers ate the file, so every \
         extraction below would find nothing and pass",
        text.len()
    );

    let vendors_in = |name: &str| -> Vec<&str> {
        LLM_VENDOR_TERMS
            .iter()
            .filter(|term| !vendor_hits(name, term).is_empty())
            .copied()
            .collect()
    };

    // Item extraction: type-like definitions and function names, whatever
    // their visibility (D-E's three methods are trait methods, not `pub fn`).
    let mut actual: BTreeSet<(String, String, String)> = BTreeSet::new();
    for (index, _) in text.match_indices("fn ") {
        let before_ok = index == 0 || {
            let previous = text.as_bytes()[index - 1];
            !(previous.is_ascii_alphanumeric() || previous == b'_')
        };
        if !before_ok {
            continue;
        }
        let name: String = text[index + 3..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        for vendor in vendors_in(&name) {
            actual.insert(("method".to_string(), name.clone(), vendor.to_string()));
        }
    }
    for keyword in ["struct ", "enum ", "trait ", "union ", "type "] {
        for (index, _) in text.match_indices(keyword) {
            let before_ok = index == 0 || {
                let previous = text.as_bytes()[index - 1];
                !(previous.is_ascii_alphanumeric() || previous == b'_')
            };
            if !before_ok {
                continue;
            }
            let name: String = text[index + keyword.len()..]
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            for vendor in vendors_in(&name) {
                actual.insert(("dto".to_string(), name.clone(), vendor.to_string()));
            }
        }
    }

    let frozen: BTreeSet<(String, String, String)> = SANCTIONED_VENDOR_API
        .iter()
        .map(|(kind, name, vendor)| (kind.to_string(), name.to_string(), vendor.to_string()))
        .collect();
    assert_eq!(
        frozen.len(),
        SANCTIONED_VENDOR_API.len(),
        "SANCTIONED_VENDOR_API holds duplicate rows"
    );

    let added: Vec<String> = actual
        .difference(&frozen)
        .map(|(kind, name, vendor)| format!("    + {kind} `{name}` (vendor `{vendor}`)"))
        .collect();
    assert!(
        added.is_empty(),
        "NEW VENDOR-NAMED ITEM IN {SANCTIONED_MODULE} (#7150). §12.11 D-E's sanction is bounded \
         at the current six DTOs and three methods: \"a *fourth* provider login must arrive as a \
         package or behind a shape that adds no vendor-named method or DTO\". A rename that \
         swaps one vendor for another lands here too, which is the point — the counts alone \
         would not notice it:\n{}",
        added.join("\n")
    );
    let removed: Vec<String> = frozen
        .difference(&actual)
        .map(|(kind, name, vendor)| format!("    - {kind} `{name}` (vendor `{vendor}`)"))
        .collect();
    assert!(
        removed.is_empty(),
        "SANCTIONED_VENDOR_API names items {SANCTIONED_MODULE} no longer defines. Delete the \
         rows and lower the ceilings in the same PR so the narrowing is banked:\n{}",
        removed.join("\n")
    );

    let dtos = actual.iter().filter(|(kind, ..)| kind == "dto").count();
    let methods = actual.iter().filter(|(kind, ..)| kind == "method").count();
    let distinct: BTreeSet<&str> = actual
        .iter()
        .map(|(_, _, vendor)| vendor.as_str())
        .collect();

    assert_eq!(
        dtos, D_E_VENDOR_DTO_CEILING,
        "{SANCTIONED_MODULE} defines {dtos} vendor-named DTOs; §12.11 D-E bounds it at \
         {D_E_VENDOR_DTO_CEILING} — \"no seventh vendor name joins the six\". Growth needs an \
         amended ruling; a reduction needs this ceiling lowered in the same PR."
    );
    assert_eq!(
        methods, D_E_VENDOR_METHOD_CEILING,
        "{SANCTIONED_MODULE} defines {methods} vendor-named methods; §12.11 D-E bounds it at \
         {D_E_VENDOR_METHOD_CEILING} — a fourth provider login must arrive as a package or \
         behind a shape that adds no vendor-named method or DTO."
    );
    assert_eq!(
        distinct.len(),
        D_E_DISTINCT_VENDOR_CEILING,
        "{SANCTIONED_MODULE} names {} distinct LLM vendors ({distinct:?}); D-E bounds it at \
         {D_E_DISTINCT_VENDOR_CEILING}. This is the strictest reading of \"no seventh vendor \
         name\" and is pinned so no reading of the ruling is left unenforced.",
        distinct.len()
    );
}

/// A carve-out must describe a live collision, or it is a hole.
#[test]
fn reborn_vendor_term_collision_carve_outs_stay_live_and_narrow() {
    assert!(
        !TERM_COLLISION_CARVE_OUTS.is_empty(),
        "TERM_COLLISION_CARVE_OUTS is empty — if the collision it held is gone, delete this \
         assertion with it; passing over an empty list is not evidence of anything"
    );
    let root = workspace_root();
    for (path, term, reason) in TERM_COLLISION_CARVE_OUTS {
        assert!(
            LLM_VENDOR_TERMS.contains(term),
            "carve-out ({path}, {term}) names a term the census does not scan for — it \
             suppresses nothing and hides that the term is unpinned"
        );
        assert!(
            reason.len() > 30,
            "carve-out ({path}, {term}) must state WHY the match is not a vendor reference"
        );
        let file = crate_path(&root, path);
        let raw = std::fs::read_to_string(&file).unwrap_or_else(|error| {
            panic!(
                "carve-out path {path} is unreadable — a carve-out for a file that moved \
                 suppresses nothing here and everything at the new path is unmeasured: {error}"
            )
        });
        let text = strip_cfg_test_blocks(&strip_comments(&raw));
        assert!(
            !vendor_hits(&text, term).is_empty(),
            "carve-out ({path}, {term}) no longer matches anything. Delete it — a stale \
             carve-out is a standing permission for the term to reappear unnoticed."
        );
    }
}

// ---------------------------------------------------------------------------
// Self-tests
// ---------------------------------------------------------------------------

#[test]
fn reborn_vendor_term_matcher_self_test() {
    // Underscore is a word separator: without this the three D-E methods are
    // invisible and the census under-reports the surface as six items.
    assert_eq!(vendor_hits("start_nearai_login", "nearai").len(), 1);
    assert_eq!(
        vendor_hits("complete_nearai_wallet_login", "nearai").len(),
        1
    );
    assert_eq!(vendor_hits("start_codex_login", "codex").len(), 1);
    // camelCase boundaries.
    assert_eq!(vendor_hits("NearAiLoginStart", "nearai").len(), 1);
    assert_eq!(vendor_hits("CodexLoginStart", "codex").len(), 1);
    // Hyphen/dot/quote boundaries, as in model ids.
    assert_eq!(vendor_hits("\"gpt-4o\"", "gpt").len(), 1);
    assert_eq!(vendor_hits("\"claude-opus-4\"", "opus").len(), 1);
    assert_eq!(vendor_hits("openai/gpt-5", "openai").len(), 1);

    // Substring collisions must NOT match.
    assert!(
        vendor_hits("ollama", "llama").is_empty(),
        "`llama` must not fire inside `ollama` — the local-model backend is not Meta's model"
    );
    assert!(
        vendor_hits("opusculum", "opus").is_empty(),
        "a longer word merely starting with the term must not match"
    );
    assert!(
        vendor_hits("microgpt", "gpt").is_empty(),
        "a term glued to a preceding lower-case word must not match"
    );
    assert!(vendor_hits("xgrok", "grok").is_empty());
    // Case insensitivity.
    assert_eq!(vendor_hits("ANTHROPIC", "anthropic").len(), 1);
    // Every occurrence is counted, not just the first.
    assert_eq!(vendor_hits("gpt-4 gpt-5 gpt-6", "gpt").len(), 3);
}

#[test]
fn reborn_vendor_census_stripper_self_test() {
    let source = r#"
/// A doc comment naming anthropic.
// A line comment naming gemini.
/* A block comment naming cohere. */
pub struct NearAiLoginStart {
    pub provider: String,
}
const ID: &str = "openai";
#[cfg(test)]
mod tests {
    const HIDDEN: &str = "mistral";
    fn helper_bedrock() {}
}
pub fn tail_after_cfg_block_groq() {}
"#;
    let stripped = strip_cfg_test_blocks(&strip_comments(source));

    for invisible in ["anthropic", "gemini", "cohere", "mistral", "bedrock"] {
        assert!(
            vendor_hits(&stripped, invisible).is_empty(),
            "`{invisible}` must be invisible: comments and #[cfg(test)] items are not \
             production vendor coupling"
        );
    }
    // Identifiers and string literals stay visible — a provider id hides in a
    // string, which is why this gate does not reuse the shared stripper that
    // blanks string contents.
    assert_eq!(
        vendor_hits(&stripped, "nearai").len(),
        1,
        "a vendor-named identifier must survive stripping"
    );
    assert_eq!(
        vendor_hits(&stripped, "openai").len(),
        1,
        "a vendor id in a STRING LITERAL must survive stripping — blanking strings is the \
         fail-open direction for a census"
    );
    // Everything after the cfg(test) block must be scanned, not swallowed.
    assert_eq!(
        vendor_hits(&stripped, "groq").len(),
        1,
        "content following a #[cfg(test)] item must still be scanned; a stripper that eats the \
         rest of the file passes by measuring nothing"
    );

    // An out-of-line `#[cfg(test)] mod tests;` declaration must not swallow the
    // remainder either.
    let out_of_line = "#[cfg(test)]\nmod tests;\nconst ID: &str = \"tinfoil\";\n";
    assert_eq!(
        vendor_hits(
            &strip_cfg_test_blocks(&strip_comments(out_of_line)),
            "tinfoil"
        )
        .len(),
        1
    );
}
