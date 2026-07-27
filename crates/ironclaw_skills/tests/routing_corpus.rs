use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use ironclaw_skills::{
    LoadedSkill, SkillSelectionOptions, SkillSource, SkillTrust, parse_skill_md,
    prefilter_skills_with_options,
};
use serde::Deserialize;

const TOP_K: usize = 5;
const EVALUATION_TOKEN_BUDGET: usize = 100_000;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutingCorpus {
    schema_version: u32,
    top_k: usize,
    cases: Vec<RoutingCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutingCase {
    id: String,
    class: RoutingCaseClass,
    prompt: String,
    relevant: Vec<String>,
    forbidden: Vec<String>,
    expect_no_match: bool,
    baseline_selected: Vec<String>,
    #[serde(default)]
    source_issue: Option<u64>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum RoutingCaseClass {
    Positive,
    Negative,
    Conflict,
    MultiSkill,
}

#[derive(Debug, Default)]
struct BaselineMetrics {
    relevant_total: usize,
    relevant_retrieved: usize,
    positive_cases: usize,
    relevant_top_one: usize,
    forbidden_cases: usize,
    forbidden_hits: usize,
    no_match_cases: usize,
    false_activations: usize,
}

#[test]
fn real_skill_routing_corpus_matches_the_reviewed_legacy_baseline() {
    let corpus: RoutingCorpus = serde_json::from_str(include_str!("fixtures/routing_corpus.json"))
        .expect("routing corpus must parse");
    assert_eq!(
        corpus.schema_version, 1,
        "unsupported routing corpus version"
    );
    assert_eq!(
        corpus.top_k, TOP_K,
        "the checked-in baseline and evaluator must use the same top-k"
    );
    assert!(
        corpus.cases.len() >= 15,
        "routing corpus must remain broad enough to cover conflicts and negatives"
    );

    let skills = load_bundled_skills();
    let skills_by_name = skills
        .iter()
        .map(|skill| (skill.name().to_string(), skill))
        .collect::<HashMap<_, _>>();
    validate_corpus(&corpus, &skills_by_name);

    let mut metrics = BaselineMetrics::default();
    for case in &corpus.cases {
        let selected = prefilter_skills_with_options(
            &case.prompt,
            &skills,
            corpus.top_k,
            EVALUATION_TOKEN_BUDGET,
            &HashSet::new(),
            SkillSelectionOptions::default(),
        )
        .selected
        .into_iter()
        .map(|skill| skill.name().to_string())
        .collect::<Vec<_>>();

        assert_eq!(
            selected, case.baseline_selected,
            "routing baseline changed for case {}; review whether this is an intentional quality change, then update the fixture",
            case.id
        );

        metrics.relevant_total += case.relevant.len();
        metrics.relevant_retrieved += case
            .relevant
            .iter()
            .filter(|name| selected.contains(name))
            .count();
        if !case.relevant.is_empty() {
            metrics.positive_cases += 1;
            if selected
                .first()
                .is_some_and(|name| case.relevant.contains(name))
            {
                metrics.relevant_top_one += 1;
            }
        }
        if !case.forbidden.is_empty() {
            metrics.forbidden_cases += 1;
            if case.forbidden.iter().any(|name| selected.contains(name)) {
                metrics.forbidden_hits += 1;
            }
        }
        if case.expect_no_match {
            metrics.no_match_cases += 1;
            if !selected.is_empty() {
                metrics.false_activations += 1;
            }
        }

        eprintln!(
            "skill-routing-baseline case={} class={:?} selected={selected:?}",
            case.id, case.class
        );
    }

    // Slice 0 records these quality measurements without enforcing the epic's
    // promotion thresholds. Slice 6 owns turning reviewed targets into gates.
    eprintln!("skill-routing-baseline metrics={metrics:?}");
}

fn load_bundled_skills() -> Vec<LoadedSkill> {
    let skills_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills");
    let mut skill_paths = std::fs::read_dir(&skills_root)
        .expect("bundled skills root must be readable")
        .map(|entry| {
            entry
                .expect("skill directory entry must be readable")
                .path()
        })
        .filter(|path| path.is_dir())
        .map(|path| path.join("SKILL.md"))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    skill_paths.sort();

    skill_paths
        .into_iter()
        .map(|path| load_bundled_skill(&path))
        .collect()
}

fn load_bundled_skill(path: &Path) -> LoadedSkill {
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("read bundled skill {}: {error}", path.display()));
    let parsed = parse_skill_md(&raw)
        .unwrap_or_else(|error| panic!("parse bundled skill {}: {error}", path.display()));
    let directory_name = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or_else(|| {
            panic!(
                "bundled skill path has no UTF-8 directory: {}",
                path.display()
            )
        });
    assert_eq!(
        parsed.manifest.name,
        directory_name,
        "bundled skill name must match its directory for {}",
        path.display()
    );
    let compiled_patterns = LoadedSkill::compile_patterns(&parsed.manifest.activation.patterns);
    let lowercased_keywords = parsed
        .manifest
        .activation
        .keywords
        .iter()
        .map(|keyword| keyword.to_lowercase())
        .collect();
    let lowercased_exclude_keywords = parsed
        .manifest
        .activation
        .exclude_keywords
        .iter()
        .map(|keyword| keyword.to_lowercase())
        .collect();
    let lowercased_tags = parsed
        .manifest
        .activation
        .tags
        .iter()
        .map(|tag| tag.to_lowercase())
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

fn validate_corpus(corpus: &RoutingCorpus, skills: &HashMap<String, &LoadedSkill>) {
    let mut ids = HashSet::new();
    let mut classes = HashSet::new();
    let mut issue_5417_cases = 0;
    for case in &corpus.cases {
        assert!(
            ids.insert(&case.id),
            "duplicate routing case id: {}",
            case.id
        );
        assert!(
            !case.prompt.trim().is_empty(),
            "routing case {} has an empty prompt",
            case.id
        );
        classes.insert(format!("{:?}", case.class));
        if case.class == RoutingCaseClass::Negative {
            assert!(
                case.relevant.is_empty(),
                "negative routing case {} must not declare relevant skills",
                case.id
            );
            assert!(
                case.expect_no_match,
                "negative routing case {} must expect no match",
                case.id
            );
        } else {
            assert!(
                !case.relevant.is_empty(),
                "non-negative routing case {} needs relevant skills",
                case.id
            );
            assert!(
                !case.expect_no_match,
                "non-negative routing case {} cannot expect no match",
                case.id
            );
        }
        for name in case
            .relevant
            .iter()
            .chain(&case.forbidden)
            .chain(&case.baseline_selected)
        {
            assert!(
                skills.contains_key(name),
                "routing case {} references missing bundled skill {name}",
                case.id
            );
        }
        assert!(
            case.relevant
                .iter()
                .all(|name| !case.forbidden.contains(name)),
            "routing case {} marks the same skill relevant and forbidden",
            case.id
        );
        if case.source_issue == Some(5417) {
            issue_5417_cases += 1;
            assert_eq!(
                case.id, "hacker-news-search-no-skill",
                "issue 5417 must identify the frozen Hacker News regression"
            );
            assert!(
                case.forbidden
                    .iter()
                    .any(|name| name == "tech-debt-tracker"),
                "issue 5417 must keep tech-debt-tracker as a forbidden activation"
            );
        }
    }
    assert_eq!(
        issue_5417_cases, 1,
        "the routing corpus must contain exactly one issue 5417 regression"
    );
    assert_eq!(
        classes.len(),
        4,
        "routing corpus must cover positive, negative, conflict, and multi-skill cases"
    );
}
