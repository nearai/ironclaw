//! Doc-fact contract: `docs/using/cli.mdx` tracks the real command surface.
//!
//! Pins both directions: every visible subcommand from `--help` has a doc
//! table row (any visible alias form counts), and every documented command
//! path — nested commands included — exists in the binary
//! (`ironclaw <path> --help` must succeed). Flag-level coverage is
//! deliberately out of scope (flags churn too fast; recorded as a
//! follow-up), and there is no zh mirror of this page.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

/// Visible aliases per subcommand, mirrored from the `#[command(visible_alias)]`
/// attributes in `src/commands/mod.rs`. `--help` prints only the primary name,
/// but the doc may legitimately document any visible form.
const VISIBLE_ALIASES: &[(&str, &[&str])] = &[("ironhub", &["iron-hub", "hub"])];

fn repo_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        if dir.join("docs/using/cli.mdx").is_file() {
            return dir;
        }
        assert!(
            dir.pop(),
            "walked out of the filesystem without finding docs/using/cli.mdx"
        );
    }
}

/// Subcommand names from the real binary's `--help`, `env_clear()`ed like
/// every other spawn in this suite. The auto-generated `help` subcommand is
/// not a documentation obligation.
fn help_subcommands() -> BTreeSet<String> {
    let output = Command::new(env!("CARGO_BIN_EXE_ironclaw"))
        .env_clear()
        .env("IRONCLAW_DISABLE_OS_KEYCHAIN", "1")
        .arg("--help")
        .output()
        .expect("spawn ironclaw --help");
    assert!(
        output.status.success(),
        "ironclaw --help failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("--help output is UTF-8");

    let mut commands = BTreeSet::new();
    let mut in_commands = false;
    for line in stdout.lines() {
        if line.trim_end() == "Commands:" {
            in_commands = true;
            continue;
        }
        if in_commands {
            if !line.starts_with("  ") || line.trim().is_empty() {
                break;
            }
            let name = line
                .split_whitespace()
                .next()
                .expect("non-empty command line");
            if name != "help" {
                commands.insert(name.to_string());
            }
        }
    }
    assert!(
        !commands.is_empty(),
        "parsed no subcommands from --help; the clap output format changed:\n{stdout}"
    );
    commands
}

/// The leading command words of every `` `ironclaw <words>` `` table row in
/// the doc — the full path up to the first flag or placeholder. Fenced usage
/// examples are not rows and are not extracted.
fn documented_command_paths(doc: &str) -> Vec<Vec<String>> {
    let mut paths = Vec::new();
    for line in doc.lines() {
        let Some(rest) = line.trim_start().strip_prefix("| `ironclaw ") else {
            continue;
        };
        let Some(command_text) = rest.split('`').next() else {
            continue;
        };
        let words: Vec<String> = command_text
            .split_whitespace()
            .take_while(|word| {
                !word.starts_with('-') && word.chars().all(|c| c.is_ascii_lowercase() || c == '-')
            })
            .map(str::to_string)
            .collect();
        if !words.is_empty() {
            paths.push(words);
        }
    }
    paths
}

fn alias_forms(subcommand: &str) -> Vec<&str> {
    let mut forms = vec![subcommand];
    for (primary, aliases) in VISIBLE_ALIASES {
        if *primary == subcommand {
            forms.extend(*aliases);
        }
    }
    forms
}

#[test]
fn cli_reference_documents_every_subcommand_and_no_retired_ones() {
    let doc_path = repo_root().join("docs/using/cli.mdx");
    let doc = std::fs::read_to_string(&doc_path).expect("read docs/using/cli.mdx");

    let real = help_subcommands();
    let documented = documented_command_paths(&doc);

    // No row-count floor: the binary's own subcommand set is the fail-closed
    // anchor — a broken extractor drops some subcommand's only row and the
    // completeness check below fails.
    let documented_first_words: BTreeSet<&str> =
        documented.iter().map(|path| path[0].as_str()).collect();
    let undocumented: Vec<&String> = real
        .iter()
        .filter(|subcommand| {
            !alias_forms(subcommand)
                .iter()
                .any(|form| documented_first_words.contains(form))
        })
        .collect();
    assert!(
        undocumented.is_empty(),
        "subcommands missing from docs/using/cli.mdx: {undocumented:?} — add a \
         `| `ironclaw <command>` | ... |` row for each (any visible alias form \
         counts); every subcommand missing means the row extractor broke",
    );

    // Every documented path must exist in the binary, nested commands
    // included: `ironclaw <path> --help` succeeds only for real commands.
    let unique_paths: BTreeSet<&[String]> = documented.iter().map(Vec::as_slice).collect();
    let mut retired = Vec::new();
    for path in unique_paths {
        let output = Command::new(env!("CARGO_BIN_EXE_ironclaw"))
            .env_clear()
            .env("IRONCLAW_DISABLE_OS_KEYCHAIN", "1")
            .args(path.iter())
            .arg("--help")
            .output()
            .expect("spawn ironclaw <path> --help");
        if !output.status.success() {
            retired.push(path.join(" "));
        }
    }
    assert!(
        retired.is_empty(),
        "docs/using/cli.mdx documents commands the binary does not have: {retired:?} — \
         delete the rows or fix the command names",
    );
}

/// The alias table above is itself a claim about the clap tree — a renamed
/// or retired alias must fail here, not silently excuse a doc row. Visible
/// aliases print in the top-level `--help` command listing as
/// `[aliases: ...]` on the primary's line.
#[test]
fn visible_alias_table_matches_the_binary() {
    let output = Command::new(env!("CARGO_BIN_EXE_ironclaw"))
        .env_clear()
        .env("IRONCLAW_DISABLE_OS_KEYCHAIN", "1")
        .arg("--help")
        .output()
        .expect("spawn ironclaw --help");
    let help = String::from_utf8(output.stdout).expect("--help output is UTF-8");
    for (primary, aliases) in VISIBLE_ALIASES {
        let line = help
            .lines()
            .find(|line| line.trim_start().starts_with(primary))
            .unwrap_or_else(|| {
                panic!("VISIBLE_ALIASES names `{primary}`, which `--help` does not list")
            });
        // Exact tokens from `[aliases: a, b]` — substring matching would let
        // a removed `hub` ride on the remaining `iron-hub`.
        let listed: BTreeSet<&str> = line
            .split_once("[aliases:")
            .map(|(_, rest)| rest.trim_end().trim_end_matches(']'))
            .unwrap_or_else(|| {
                panic!("`--help` lists no `[aliases: ...]` on the `{primary}` line: {line}")
            })
            .split(',')
            .map(str::trim)
            .collect();
        for alias in *aliases {
            assert!(
                listed.contains(alias),
                "`--help` does not list visible alias `{alias}` on the `{primary}` line; \
                 update VISIBLE_ALIASES to match src/commands/mod.rs: {line}"
            );
        }
    }
}
