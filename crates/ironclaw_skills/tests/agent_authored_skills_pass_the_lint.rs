//! The descriptor lint must not block self-creation.
//!
//! `lint_skill_routing_metadata` gates `parse_distillation` and `parse_refinement`, so a learned
//! skill that fails it is never written. That is the point — a skill declaring `file` as a keyword
//! degrades routing for every later request. But it puts the lint directly in the path of the
//! behaviour this epic exists to enable: if real agent-authored skills routinely fail it,
//! self-creation stops working entirely and the failure is silent (a `DistillError`, no skill on
//! disk).
//!
//! The self-creation measurement could not catch this. It re-ran only the *use* phase against
//! skills authored by an earlier build, which isolates routing but never exercises the write path
//! the lint guards.
//!
//! So this lints the real corpus: the skills agents actually wrote during the self-creation runs,
//! read from the persistent stores. Skipped cleanly when those stores are absent.
//!
//!     NEARAI_SELFCREATE_STORES=~/.cache/nearai-bench/persistent-agent-state \
//!       cargo test -p ironclaw_skills --test agent_authored_skills_pass_the_lint -- --nocapture

use std::path::PathBuf;

use ironclaw_skills::{
    lint_skill_routing_metadata_advisory, lint_skill_routing_metadata_blocking, parse_skill_md,
};

fn stores_root() -> PathBuf {
    if let Ok(dir) = std::env::var("NEARAI_SELFCREATE_STORES") {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".cache/nearai-bench/persistent-agent-state")
}

/// Every `us31-*/skills/*/SKILL.md`: one skill per task, authored by the agent itself.
fn agent_authored() -> Vec<(String, String)> {
    let root = stores_root();
    let mut out = Vec::new();
    let Ok(stores) = std::fs::read_dir(&root) else {
        return out;
    };
    for store in stores.flatten() {
        let name = store.file_name().to_string_lossy().to_string();
        if !name.starts_with("us31-") {
            continue;
        }
        let skills = store.path().join("skills");
        let Ok(entries) = std::fs::read_dir(&skills) else {
            continue;
        };
        for entry in entries.flatten() {
            let md = entry.path().join("SKILL.md");
            if !md.is_file() {
                continue;
            }
            if let Ok(raw) = std::fs::read_to_string(&md) {
                out.push((
                    entry.file_name().to_string_lossy().to_string(),
                    raw,
                ));
            }
        }
    }
    out
}

#[test]
fn the_lint_does_not_block_skills_agents_actually_write() {
    let skills = agent_authored();
    if skills.is_empty() {
        eprintln!(
            "no agent-authored skill stores under {}; skipping",
            stores_root().display()
        );
        return;
    }

    let mut blocked: Vec<(String, Vec<String>)> = Vec::new();
    let mut unparseable: Vec<String> = Vec::new();
    let mut no_description = 0usize;
    let mut advisory_count = 0usize;

    for (name, raw) in &skills {
        let Ok(parsed) = parse_skill_md(raw) else {
            unparseable.push(name.clone());
            continue;
        };
        if parsed.manifest.description.trim().is_empty() {
            no_description += 1;
        }
        // Only the BLOCKING rules can refuse a write; advisory ones warn.
        let problems = lint_skill_routing_metadata_blocking(&parsed.manifest);
        if !problems.is_empty() {
            blocked.push((name.clone(), problems));
        }
        advisory_count += lint_skill_routing_metadata_advisory(&parsed.manifest).len();
    }

    println!(
        "\n=== {} agent-authored skills linted ===",
        skills.len() - unparseable.len()
    );
    println!("  would be REFUSED by the write gate: {}", blocked.len());
    println!("  advisory warnings (do NOT block): {advisory_count}");
    println!("  no description at all:        {no_description}");
    if !unparseable.is_empty() {
        println!("  unparseable:                 {unparseable:?}");
    }
    for (name, problems) in &blocked {
        println!("\n  {name}");
        for problem in problems {
            println!("     - {problem}");
        }
    }
    println!();

    assert!(
        blocked.is_empty(),
        "{} of {} agent-authored skills would be refused by the descriptor lint, which gates the \
         learned-skill write path -- self-creation would silently stop producing skills. Either \
         the lint rule is too strict for authored skills, or the authoring prompt must be taught \
         to satisfy it.",
        blocked.len(),
        skills.len()
    );
}
