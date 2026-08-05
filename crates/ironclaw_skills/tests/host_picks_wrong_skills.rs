//! What the HOST picks, with no model involved — the wrong-skill failure, measured exactly.
//!
//! Two things could activate a skill before this work:
//!
//!   1. the model asks, via `builtin.skill_activate`
//!   2. the **host** decides, by scoring each skill's declared `activation:` keywords against
//!      the user's message during prompt assembly, and injecting the winners
//!
//! #5417 is (2): `tech-debt-tracker` declares the keyword `hack`, "search Hacker News" matches
//! it as a substring, and a technical-debt skill loads that nobody asked for.
//!
//! The benchmark could not see (2). Its scorer reads `skill_activate` tool calls, and a host
//! pick produces no tool call — so the pre-change arm scored 100% "precision" while the host
//! was picking freely. That was an invisible failure mode, not clean routing.
//!
//! This measures it directly instead, and needs no benchmark run at all: host selection is
//! `prefilter_skills_with_options`, a pure deterministic function of (message, skill metadata).
//! No LLM, no logs, no attribution guesswork. Every task's real prompt is scored against the
//! real 227-skill catalog and the selection is compared to that task's expected set.
//!
//! Run with output:
//!     cargo test -p ironclaw_skills --test host_picks_wrong_skills -- --nocapture

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use ironclaw_skills::{
    LoadedSkill, SkillSelectionOptions, SkillSource, SkillTrust, parse_skill_md,
    prefilter_skills_with_options,
};

/// Mirrors reborn's local-dev composition, so the numbers describe the shipped configuration.
const MAX_ACTIVE_SKILLS: usize = 8;
const MAX_CONTEXT_TOKENS: usize = 6000;

fn bench_root() -> PathBuf {
    // The catalog and task prompts live in the benchmarks repo, checked out alongside ironclaw.
    // Overridable so this runs from any layout; skipped cleanly when absent.
    if let Ok(dir) = std::env::var("NEARAI_BENCH_ROOT") {
        return PathBuf::from(dir);
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../nearai/benchmarks")
}

/// Build the runtime shape the scorer consumes, exactly as `routing_corpus.rs` does — the
/// lowercased keyword/tag caches and compiled patterns are what `score_skill` reads, so
/// constructing them by hand here would measure a different scorer than the one that ships.
fn into_loaded(parsed: ironclaw_skills::ParsedSkill, path: &Path) -> LoadedSkill {
    let compiled_patterns = LoadedSkill::compile_patterns(&parsed.manifest.activation.patterns);
    let lowercased_keywords = parsed
        .manifest
        .activation
        .keywords
        .iter()
        .map(|k| k.to_lowercase())
        .collect();
    let lowercased_exclude_keywords = parsed
        .manifest
        .activation
        .exclude_keywords
        .iter()
        .map(|k| k.to_lowercase())
        .collect();
    let lowercased_tags = parsed
        .manifest
        .activation
        .tags
        .iter()
        .map(|t| t.to_lowercase())
        .collect();
    LoadedSkill {
        manifest: parsed.manifest,
        prompt_content: parsed.prompt_content,
        trust: SkillTrust::Trusted,
        source: SkillSource::Bundled(PathBuf::from(path)),
        content_hash: String::new(),
        compiled_patterns,
        lowercased_keywords,
        lowercased_exclude_keywords,
        lowercased_tags,
    }
}

fn load_catalog(root: &Path) -> Vec<LoadedSkill> {
    let mut out = Vec::new();
    let mut roots = vec![
        root.join("datasets/pinchbench/v1/routing_catalog/canonical"),
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills"),
    ];
    while let Some(dir) = roots.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let md = path.join("SKILL.md");
            if !md.is_file() {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(&md) else {
                continue;
            };
            if let Ok(parsed) = parse_skill_md(&raw) {
                out.push(into_loaded(parsed, &md));
            }
        }
    }
    out
}

/// The prompt the model actually sees: the task body with the domain-naming hint stripped, which
/// is what `BENCH_SKILL_ROUTING=1` does. Leaving the hint in would hand the scorer its answer.
fn task_prompt(root: &Path, task_id: &str) -> Option<String> {
    let dir = root.join("datasets/pinchbench/v1/tasks");
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name()?.to_string_lossy().to_string();
        // Files are `<task_id>.md` or `<task_id>_selfcreate.md`. Matching the id exactly avoids
        // the fuzzy `upstream`-substring lookup, which silently missed 7 of 31 tasks.
        let stem = name.strip_suffix(".md")?;
        if stem != task_id && stem != format!("{task_id}_selfcreate") {
            continue;
        }
        let raw = std::fs::read_to_string(&path).ok()?;
        let body = raw.split("---").nth(2).unwrap_or(&raw).to_string();
        return Some(
            body.lines()
                .filter(|line| !line.contains("check your skills catalog"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    None
}

#[test]
fn the_host_keyword_scorer_picks_skills_the_task_does_not_need() {
    let root = bench_root();
    if !root
        .join("datasets/pinchbench/v1/routing_catalog/manifest.json")
        .is_file()
    {
        eprintln!(
            "benchmarks checkout not found at {}; skipping",
            root.display()
        );
        return;
    }
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("datasets/pinchbench/v1/routing_catalog/manifest.json"))
            .expect("read manifest"),
    )
    .expect("parse manifest");

    let catalog = load_catalog(&root);
    assert!(
        catalog.len() >= 200,
        "expected the full catalog, loaded {}",
        catalog.len()
    );

    let tasks = manifest["tasks"].as_object().expect("tasks object");
    let markers: HashSet<String> = HashSet::new();
    let mut scored = 0usize;
    let mut total_picked = 0usize;
    let mut total_correct = 0usize;
    let mut tasks_with_wrong = 0usize;
    let mut worst: Vec<(String, usize, usize, Vec<String>)> = Vec::new();
    let mut offender_counts: HashMap<String, usize> = HashMap::new();

    for (task_id, info) in tasks {
        let expected: HashSet<String> = info["expected"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let Some(prompt) = task_prompt(&root, task_id) else {
            eprintln!("  no prompt file for {task_id}");
            continue;
        };
        let outcome = prefilter_skills_with_options(
            &prompt,
            &catalog,
            MAX_ACTIVE_SKILLS,
            MAX_CONTEXT_TOKENS,
            &markers,
            SkillSelectionOptions::default(),
        );
        let picked: Vec<String> = outcome
            .selected
            .iter()
            .map(|skill| skill.name().to_string())
            .collect();
        scored += 1;
        total_picked += picked.len();
        let correct = picked.iter().filter(|n| expected.contains(*n)).count();
        total_correct += correct;
        let wrong: Vec<String> = picked
            .iter()
            .filter(|n| !expected.contains(*n))
            .cloned()
            .collect();
        if !wrong.is_empty() {
            tasks_with_wrong += 1;
            for w in &wrong {
                *offender_counts.entry(w.clone()).or_default() += 1;
            }
        }
        worst.push((task_id.clone(), picked.len(), correct, wrong));
    }

    assert!(
        scored >= 25,
        "scored only {scored} tasks; prompt lookup broke"
    );

    // The finding, pinned. Measured: 83 host picks across 27 tasks, ZERO correct, every task
    // getting at least one wrong skill. It is 0% rather than merely poor because the only skills
    // carrying `activation:` metadata are ironclaw's own bundled 32, and none of those is ever a
    // benchmark task's expected skill -- so every host pick is necessarily wrong AND it consumes
    // the activation slots the task's real skills needed.
    //
    // Asserted as an upper bound rather than an equality so the numbers can drift with the
    // catalog without a spurious failure, while a return to host-side picking still trips it.
    assert!(
        total_correct * 20 < total_picked,
        "host keyword picks were {total_correct}/{total_picked} correct; this test exists because \
         that ratio was 0/83 -- if the scorer has become accurate, re-examine whether \
         model-decided selection is still the right default"
    );
    assert!(
        tasks_with_wrong * 4 >= scored * 3,
        "only {tasks_with_wrong}/{scored} tasks got a wrong host pick; measured 27/27"
    );

    println!(
        "\n=== HOST keyword scorer, {scored} tasks, {} skills ===",
        catalog.len()
    );
    println!(
        "  picked {total_picked} skills, {total_correct} correct, {} WRONG",
        total_picked - total_correct
    );
    println!(
        "  precision over host picks   {:.1}%",
        if total_picked == 0 {
            0.0
        } else {
            100.0 * total_correct as f64 / total_picked as f64
        }
    );
    println!(
        "  tasks with >=1 wrong pick   {tasks_with_wrong}/{scored} ({:.0}%)",
        100.0 * tasks_with_wrong as f64 / scored as f64
    );

    let mut offenders: Vec<_> = offender_counts.into_iter().collect();
    offenders.sort_by(|a, b| b.1.cmp(&a.1));
    println!("\n  skills the host wrongly picked most often:");
    for (name, count) in offenders.iter().take(10) {
        println!("    {count:2} tasks  {name}");
    }

    // Machine-readable per-task line, so a caller can restrict to any task subset.
    println!("\n  PERTASK task_id picked correct");
    for (task, picked, correct, _) in &worst {
        println!("  PERTASK {task} {picked} {correct}");
    }

    worst.sort_by(|a, b| b.3.len().cmp(&a.3.len()));
    println!("\n  worst tasks:");
    for (task, picked, correct, wrong) in worst.iter().take(6) {
        println!(
            "    {:44} picked {picked}, {correct} correct, wrong={:?}",
            task.replace("task_skillsbench_", ""),
            wrong
        );
    }
    println!();
}
