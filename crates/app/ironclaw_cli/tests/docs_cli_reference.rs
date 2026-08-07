//! Doc-fact contract: `docs/using/cli.mdx` tracks the real command surface.
//!
//! The public CLI reference is hand-written, and nothing coupled it to the
//! clap tree until now — a renamed or added subcommand silently left the
//! published page describing a binary that does not exist (issue #7317's
//! drift class). This test pins the two directions at subcommand
//! granularity:
//!
//!   1. every visible subcommand of the real binary's `--help` appears in at
//!      least one `` `ironclaw <command>` `` table row of the doc (any
//!      visible alias form counts);
//!   2. every `` `ironclaw <command>` `` the doc documents is a real
//!      subcommand or visible alias (a documented-but-deleted command fails).
//!
//! Flag-level coverage is deliberately out of scope — flags churn faster
//! than commands and a flag gate would train people to stop reading
//! failures; the doc-truth design doc records it as a follow-up.
//!
//! There is no zh mirror of this page (`docs/zh/` has no `using/` tree), so
//! only the English page is pinned.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

/// Visible aliases per subcommand, mirrored from the `#[command(visible_alias)]`
/// attributes in `src/commands/mod.rs`. `--help` prints only the primary name,
/// but the doc may legitimately document any visible form.
const VISIBLE_ALIASES: &[(&str, &[&str])] = &[("ironhub", &["iron-hub", "hub"])];

/// Fail-closed floor: the doc currently carries 30 command rows. A parse that
/// finds fewer than this many means the table format changed, not that the
/// CLI shrank — refuse rather than verify almost nothing.
const MIN_DOC_COMMAND_ROWS: usize = 15;

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

/// The first command word of every `` `ironclaw <words>` `` table row in the
/// doc. Fenced usage examples are not rows and are not extracted.
fn documented_commands(doc: &str) -> Vec<String> {
    let mut words = Vec::new();
    for line in doc.lines() {
        let Some(rest) = line.trim_start().strip_prefix("| `ironclaw ") else {
            continue;
        };
        let Some(command_text) = rest.split('`').next() else {
            continue;
        };
        if let Some(first) = command_text.split_whitespace().next()
            && first.chars().all(|c| c.is_ascii_lowercase() || c == '-')
        {
            words.push(first.to_string());
        }
    }
    words
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
    let documented = documented_commands(&doc);
    assert!(
        documented.len() >= MIN_DOC_COMMAND_ROWS,
        "extracted only {} `ironclaw <command>` rows from {} (floor is {}); \
         the table format changed — fix the extractor, not the doc",
        documented.len(),
        doc_path.display(),
        MIN_DOC_COMMAND_ROWS,
    );

    let documented_set: BTreeSet<&str> = documented.iter().map(String::as_str).collect();

    let undocumented: Vec<&String> = real
        .iter()
        .filter(|subcommand| {
            !alias_forms(subcommand)
                .iter()
                .any(|form| documented_set.contains(form))
        })
        .collect();
    assert!(
        undocumented.is_empty(),
        "subcommands missing from docs/using/cli.mdx: {undocumented:?} — add a \
         `| `ironclaw <command>` | ... |` row for each (any visible alias form counts)",
    );

    let known_forms: BTreeSet<&str> = real
        .iter()
        .flat_map(|subcommand| alias_forms(subcommand))
        .collect();
    let retired: Vec<&&str> = documented_set
        .iter()
        .filter(|word| !known_forms.contains(**word))
        .collect();
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
        for alias in *aliases {
            assert!(
                line.contains(alias),
                "`--help` does not list visible alias `{alias}` on the `{primary}` line; \
                 update VISIBLE_ALIASES to match src/commands/mod.rs: {line}"
            );
        }
    }
}
