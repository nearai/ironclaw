//! Recorded-trace scaffold for the project-setup automation-workflow benchmarks.
//!
//! Source scenarios:
//! `nearai/benchmarks/datasets/automation-workflows/v1/project-setup/`.
//!
//! Three tiers mirror `tests/reborn_qa_recorded_behavior.rs`:
//!
//! 1. **Recorder tests** (`#[ignore]`): seed the benchmark world
//!    (`SOUL.md` as scenario identity plus workspace memory documents), drive
//!    the benchmark turn(s) through the real Reborn runtime, real Anthropic,
//!    and real tools, then flush fixtures under
//!    `tests/fixtures/llm_traces/reborn_qa/project_setup/`.
//! 2. **Contract tests** (`#[ignore]` until fixtures exist): parse the fixture
//!    and pin the benchmark's required and forbidden tool choices plus key
//!    memory-write arguments.
//! 3. **Replay tests** (`#[ignore]` until fixtures exist): replay the fixture
//!    through the real Reborn runtime and assert the resulting memory files.
//!
//! Record all seven fixtures with:
//!
//! ```bash
//! ANTHROPIC_API_KEY=... GITHUB_TOKEN=... \
//!   cargo test --test reborn_qa_project_setup record_ \
//!     -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! Env by scenario:
//! - `ANTHROPIC_API_KEY`: all scenarios.
//! - `GITHUB_TOKEN`: `bind-repo-and-workflow` when the live run exercises the
//!   GitHub surface while installing `github-workflow`.
//! - Google credentials: not required for these project-setup scenarios.
//!
//! Do not commit fabricated fixtures. A human records and scrubs them with
//! live keys after this scaffold lands.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use std::sync::Arc;

use reborn_support::{
    model_replay::RebornTraceReplayModelGateway,
    qa_trace::{
        QaTraceScenarioSetup, QaTraceSetupDocument, build_qa_trace_runtime_with_http_exchanges,
        load_qa_trace, read_qa_memory_document, record_qa_scenario, recorded_tool_calls,
        seed_qa_trace_world, send_qa_turns, strip_expected_tool_results,
    },
};
use support::trace_llm::{LlmTrace, TraceResponse};

struct ExpectedDocument {
    path: &'static str,
    fragments: &'static [&'static str],
}

struct ProjectSetupScenario {
    fixture: &'static str,
    setup: QaTraceScenarioSetup<'static>,
    turns: &'static [&'static str],
    tools_used: &'static [&'static str],
    tools_not_used: &'static [&'static str],
    response_contains: &'static [&'static str],
    expected_documents: &'static [ExpectedDocument],
}

const PROJECT_SETUP_TOOLS_USED: &[&str] = &["builtin.memory_write"];
const PROJECT_SETUP_TOOLS_NOT_USED: &[&str] = &["gmail", "slack", "google-calendar"];

const ACCEPT_SETUP_DOCS: &[QaTraceSetupDocument<'static>] = &[QaTraceSetupDocument {
    path: "projects/README.md",
    content: "# Projects\nOne directory per project. Spec lives at spec.yaml.",
}];
const ACCEPT_EXPECTED: &[ExpectedDocument] = &[ExpectedDocument {
    path: "projects/realtime-notifications/spec.yaml",
    fragments: &[
        "Realtime Notifications",
        "realtime-notifications",
        "websocket",
        "alex",
        "priya",
        "P95",
        "500ms",
    ],
}];
const ACCEPT_PROJECT_BRIEF: ProjectSetupScenario = ProjectSetupScenario {
    fixture: "project_setup/accept-project-brief",
    setup: QaTraceScenarioSetup {
        identity_prompt: Some(
            "You are a project-setup assistant. Convert any user brief into \
             projects/<slug>/spec.yaml with fields: name, slug (kebab-case), \
             scope (1-3 sentences), stakeholders[] (each with name, role), \
             target_date, success_criteria[]. If any required field is \
             missing, list it under 'open_questions' inside the spec - do not \
             invent.",
        ),
        memory_documents: ACCEPT_SETUP_DOCS,
    },
    turns: &[
        "Start a project called 'Realtime Notifications'. Goal: ship websocket-based push notifications in the web app by end of Q3. Stakeholders: me (PM), alex (eng lead), priya (design). Success: P95 delivery latency < 500ms, opt-in rate > 30%.",
    ],
    tools_used: PROJECT_SETUP_TOOLS_USED,
    tools_not_used: PROJECT_SETUP_TOOLS_NOT_USED,
    response_contains: &["Realtime Notifications", "alex", "priya", "P95", "500ms"],
    expected_documents: ACCEPT_EXPECTED,
};

const BIND_SETUP_DOCS: &[QaTraceSetupDocument<'static>] = &[QaTraceSetupDocument {
    path: "projects/realtime-notifications/spec.yaml",
    content: "name: Realtime Notifications\nslug: realtime-notifications\ntype: dev\n",
}];
const BIND_EXPECTED: &[ExpectedDocument] = &[
    ExpectedDocument {
        path: "projects/realtime-notifications/spec.yaml",
        fragments: &["repo:", "integrations/github.yaml"],
    },
    ExpectedDocument {
        path: "projects/realtime-notifications/integrations/github.yaml",
        fragments: &["acme/web-app", "main", "github-workflow"],
    },
];
const BIND_REPO_AND_WORKFLOW: ProjectSetupScenario = ProjectSetupScenario {
    fixture: "project_setup/bind-repo-and-workflow",
    setup: QaTraceScenarioSetup {
        identity_prompt: Some(
            "You are a project-setup assistant. For dev projects, write \
             projects/<slug>/integrations/github.yaml with repo (owner/name), \
             default_branch, and workflows[] = ['github-workflow']. Also \
             append to projects/<slug>/spec.yaml a `repo:` field pointing at \
             the integration file.",
        ),
        memory_documents: BIND_SETUP_DOCS,
    },
    turns: &[
        "Bind realtime-notifications to repo acme/web-app on default branch main and install github-workflow.",
    ],
    tools_used: PROJECT_SETUP_TOOLS_USED,
    tools_not_used: PROJECT_SETUP_TOOLS_NOT_USED,
    response_contains: &["acme/web-app", "main", "github-workflow"],
    expected_documents: BIND_EXPECTED,
};

const TRACKER_SETUP_DOCS: &[QaTraceSetupDocument<'static>] = &[QaTraceSetupDocument {
    path: "projects/realtime-notifications/spec.yaml",
    content: "name: Realtime Notifications\nslug: realtime-notifications\n",
}];
const TRACKER_EXPECTED: &[ExpectedDocument] = &[
    ExpectedDocument {
        path: "projects/realtime-notifications/spec.yaml",
        fragments: &[
            "tracker:",
            "linear",
            "linear.app/acme/project/realtime-notifications-abc123",
            "REA-12",
        ],
    },
    ExpectedDocument {
        path: "projects/realtime-notifications/integrations/tracker.yaml",
        fragments: &[
            "linear",
            "linear.app/acme/project/realtime-notifications-abc123",
            "REA-12",
            "access_method",
        ],
    },
];
const CREATE_TRACKER_HOME: ProjectSetupScenario = ProjectSetupScenario {
    fixture: "project_setup/create-tracker-home",
    setup: QaTraceScenarioSetup {
        identity_prompt: Some(
            "You are a project-setup assistant. When binding a tracker: (1) \
             append a `tracker:` block to projects/<slug>/spec.yaml with \
             provider, url, id; (2) write \
             projects/<slug>/integrations/tracker.yaml with the same plus \
             access_method (api/web). Never invent a tracker URL - only record \
             what the user gave you.",
        ),
        memory_documents: TRACKER_SETUP_DOCS,
    },
    turns: &[
        "Bind the realtime-notifications project to Linear: https://linear.app/acme/project/realtime-notifications-abc123, id REA-12.",
    ],
    tools_used: PROJECT_SETUP_TOOLS_USED,
    tools_not_used: PROJECT_SETUP_TOOLS_NOT_USED,
    response_contains: &[
        "linear.app/acme/project/realtime-notifications-abc123",
        "REA-12",
    ],
    expected_documents: TRACKER_EXPECTED,
};

const STRUCTURE_SETUP_DOCS: &[QaTraceSetupDocument<'static>] = &[QaTraceSetupDocument {
    path: "projects/realtime-notifications/spec.yaml",
    content: "name: Realtime Notifications\nslug: realtime-notifications\nscope: ship websocket push notifications in web by end of Q3\nstakeholders:\n  - {name: sam, role: PM}\n  - {name: alex, role: eng-lead}\n  - {name: priya, role: design}\n",
}];
const STRUCTURE_EXPECTED: &[ExpectedDocument] = &[
    ExpectedDocument {
        path: "projects/realtime-notifications/README.md",
        fragments: &["spec.yaml"],
    },
    ExpectedDocument {
        path: "projects/realtime-notifications/commitments/open/.keep",
        fragments: &[],
    },
    ExpectedDocument {
        path: "projects/realtime-notifications/commitments/resolved/.keep",
        fragments: &[],
    },
    ExpectedDocument {
        path: "projects/realtime-notifications/decisions/.keep",
        fragments: &[],
    },
    ExpectedDocument {
        path: "projects/realtime-notifications/meetings/.keep",
        fragments: &[],
    },
    ExpectedDocument {
        path: "projects/realtime-notifications/notes/.keep",
        fragments: &[],
    },
];
const CREATE_WORKSPACE_STRUCTURE: ProjectSetupScenario = ProjectSetupScenario {
    fixture: "project_setup/create-workspace-structure",
    setup: QaTraceScenarioSetup {
        identity_prompt: Some(
            "You are a project-setup assistant. The standard scaffold under \
             projects/<slug>/ is: README.md (one-line description + links), \
             commitments/open/.keep, commitments/resolved/.keep, \
             decisions/.keep, meetings/.keep, notes/.keep. README must link \
             to spec.yaml.",
        ),
        memory_documents: STRUCTURE_SETUP_DOCS,
    },
    turns: &["Scaffold the workspace for the realtime-notifications project."],
    tools_used: PROJECT_SETUP_TOOLS_USED,
    tools_not_used: PROJECT_SETUP_TOOLS_NOT_USED,
    response_contains: &["README", "commitments", "decisions", "meetings", "notes"],
    expected_documents: STRUCTURE_EXPECTED,
};

const DASHBOARD_SETUP_DOCS: &[QaTraceSetupDocument<'static>] = &[
    QaTraceSetupDocument {
        path: "projects/realtime-notifications/spec.yaml",
        content: "name: Realtime Notifications\nslug: realtime-notifications\n",
    },
    QaTraceSetupDocument {
        path: "dashboard/README.md",
        content: "# Dashboard\nWidget configs in widgets.",
    },
];
const DASHBOARD_EXPECTED: &[ExpectedDocument] = &[ExpectedDocument {
    path: "dashboard/widgets/realtime-notifications.yaml",
    fragments: &[
        "realtime-notifications",
        "status_card",
        "open_commitments",
        "last_update",
        "owner",
        "15",
    ],
}];
const DASHBOARD_WIDGET: ProjectSetupScenario = ProjectSetupScenario {
    fixture: "project_setup/dashboard-widget",
    setup: QaTraceScenarioSetup {
        identity_prompt: Some(
            "You are a project-setup assistant. A dashboard widget is a YAML \
             file at dashboard/widgets/<slug>.yaml with: title, project_slug, \
             type (status_card), metrics[] (open_commitments, last_update, \
             owner), refresh_minutes. Default refresh_minutes=15.",
        ),
        memory_documents: DASHBOARD_SETUP_DOCS,
    },
    turns: &["Add a dashboard widget for realtime-notifications."],
    tools_used: PROJECT_SETUP_TOOLS_USED,
    tools_not_used: PROJECT_SETUP_TOOLS_NOT_USED,
    response_contains: &[
        "realtime-notifications",
        "open_commitments",
        "last_update",
        "owner",
    ],
    expected_documents: DASHBOARD_EXPECTED,
};

const ROUTINE_SETUP_DOCS: &[QaTraceSetupDocument<'static>] = &[QaTraceSetupDocument {
    path: "projects/realtime-notifications/spec.yaml",
    content: "name: Realtime Notifications\nslug: realtime-notifications\ntimezone: America/Los_Angeles\n",
}];
const ROUTINE_EXPECTED: &[ExpectedDocument] = &[
    ExpectedDocument {
        path: "projects/realtime-notifications/routines/weekly-status.yaml",
        fragments: &["weekly-status", "#realtime-notifications", "0 9", "enabled"],
    },
    ExpectedDocument {
        path: "projects/realtime-notifications/routines/standup-digest.yaml",
        fragments: &[
            "standup-digest",
            "#realtime-notifications",
            "0 10",
            "enabled",
        ],
    },
];
const INSTALL_RECURRING_ROUTINES: ProjectSetupScenario = ProjectSetupScenario {
    fixture: "project_setup/install-recurring-routines",
    setup: QaTraceScenarioSetup {
        identity_prompt: Some(
            "You are a project-setup assistant. Routines live under \
             projects/<slug>/routines/<name>.yaml with: name, cron (5-field), \
             action, channel, enabled (default true). Use standard cron syntax \
             in the project's local TZ.",
        ),
        memory_documents: ROUTINE_SETUP_DOCS,
    },
    turns: &[
        "Install two routines for realtime-notifications: 'weekly-status' Mondays 9am posting to #realtime-notifications, and 'standup-digest' weekdays 10am posting to the same channel.",
    ],
    tools_used: PROJECT_SETUP_TOOLS_USED,
    tools_not_used: PROJECT_SETUP_TOOLS_NOT_USED,
    response_contains: &["weekly-status", "standup-digest", "#realtime-notifications"],
    expected_documents: ROUTINE_EXPECTED,
};

const STAKEHOLDER_SETUP_DOCS: &[QaTraceSetupDocument<'static>] = &[QaTraceSetupDocument {
    path: "projects/realtime-notifications/spec.yaml",
    content: "name: Realtime Notifications\nslug: realtime-notifications\n",
}];
const STAKEHOLDER_EXPECTED: &[ExpectedDocument] = &[
    ExpectedDocument {
        path: "projects/realtime-notifications/stakeholders/sam-acme-com.yaml",
        fragments: &["sam@acme.com", "PM", "slack", "email", "daily"],
    },
    ExpectedDocument {
        path: "projects/realtime-notifications/stakeholders/alex-acme-com.yaml",
        fragments: &["alex@acme.com", "eng", "slack", "weekly"],
    },
    ExpectedDocument {
        path: "projects/realtime-notifications/stakeholders/priya-acme-com.yaml",
        fragments: &["priya@acme.com", "design", "email", "weekly"],
    },
];
const REGISTER_STAKEHOLDERS: ProjectSetupScenario = ProjectSetupScenario {
    fixture: "project_setup/register-stakeholders",
    setup: QaTraceScenarioSetup {
        identity_prompt: Some(
            "You are a project-setup assistant. Persist each stakeholder as \
             projects/<slug>/stakeholders/<email-slug>.yaml with: name, email, \
             role, notification_channels[] (slack/email), digest_frequency \
             (daily/weekly/none). Default digest_frequency is weekly for \
             non-PMs.",
        ),
        memory_documents: STAKEHOLDER_SETUP_DOCS,
    },
    turns: &[
        "Register stakeholders for realtime-notifications: sam@acme.com (PM, slack+email daily), alex@acme.com (eng lead, slack weekly), priya@acme.com (design, email weekly).",
    ],
    tools_used: PROJECT_SETUP_TOOLS_USED,
    tools_not_used: PROJECT_SETUP_TOOLS_NOT_USED,
    response_contains: &[
        "sam@acme.com",
        "alex@acme.com",
        "priya@acme.com",
        "daily",
        "weekly",
    ],
    expected_documents: STAKEHOLDER_EXPECTED,
};

macro_rules! project_setup_tests {
    ($record:ident, $contract:ident, $replay:ident, $case:expr) => {
        #[tokio::test]
        #[ignore = "records against live Anthropic/tools; set ANTHROPIC_API_KEY and run explicitly"]
        async fn $record() {
            let case = &$case;
            record_qa_scenario(case.fixture, &case.setup, case.turns).await;
        }

        #[tokio::test]
        #[ignore = "requires a human-recorded project-setup fixture"]
        async fn $contract() {
            assert_project_setup_contract(&$case);
        }

        #[tokio::test]
        #[ignore = "requires a human-recorded project-setup fixture"]
        async fn $replay() {
            replay_project_setup_scenario(&$case).await;
        }
    };
}

project_setup_tests!(
    record_accept_project_brief,
    contract_accept_project_brief,
    replay_accept_project_brief,
    ACCEPT_PROJECT_BRIEF
);
project_setup_tests!(
    record_bind_repo_and_workflow,
    contract_bind_repo_and_workflow,
    replay_bind_repo_and_workflow,
    BIND_REPO_AND_WORKFLOW
);
project_setup_tests!(
    record_create_tracker_home,
    contract_create_tracker_home,
    replay_create_tracker_home,
    CREATE_TRACKER_HOME
);
project_setup_tests!(
    record_create_workspace_structure,
    contract_create_workspace_structure,
    replay_create_workspace_structure,
    CREATE_WORKSPACE_STRUCTURE
);
project_setup_tests!(
    record_dashboard_widget,
    contract_dashboard_widget,
    replay_dashboard_widget,
    DASHBOARD_WIDGET
);
project_setup_tests!(
    record_install_recurring_routines,
    contract_install_recurring_routines,
    replay_install_recurring_routines,
    INSTALL_RECURRING_ROUTINES
);
project_setup_tests!(
    record_register_stakeholders,
    contract_register_stakeholders,
    replay_register_stakeholders,
    REGISTER_STAKEHOLDERS
);

fn final_text_reply(trace: &LlmTrace) -> Option<String> {
    trace
        .turns
        .iter()
        .flat_map(|turn| turn.steps.iter())
        .rev()
        .find_map(|step| match &step.response {
            TraceResponse::Text { content, .. } => Some(content.clone()),
            _ => None,
        })
}

fn assert_project_setup_contract(case: &ProjectSetupScenario) {
    let trace = load_qa_trace(case.fixture);
    let calls = recorded_tool_calls(&trace);
    for tool in case.tools_used {
        assert!(
            calls.iter().any(|(name, _)| name == *tool),
            "expected {tool} to be used for {}; recorded calls: {calls:#?}",
            case.fixture
        );
    }
    for forbidden in case.tools_not_used {
        assert!(
            calls
                .iter()
                .all(|(name, arguments)| { !forbidden_tool_matches(name, arguments, forbidden) }),
            "expected {forbidden} not to be used for {}; recorded calls: {calls:#?}",
            case.fixture
        );
    }
    for document in case.expected_documents {
        assert_memory_write_for_document(case, &calls, document);
    }
    let reply = final_text_reply(&trace).unwrap_or_else(|| {
        panic!(
            "project-setup fixture {} should end with a text reply",
            case.fixture
        )
    });
    for fragment in case.response_contains {
        assert!(
            reply.contains(fragment),
            "reply for {} should contain {fragment:?}; reply: {reply:?}",
            case.fixture
        );
    }
}

fn forbidden_tool_matches(name: &str, arguments: &str, forbidden: &str) -> bool {
    let legacy = forbidden.replace('-', "_");
    name == forbidden
        || name == legacy
        || name.starts_with(&format!("{forbidden}."))
        || name.starts_with(&format!("{legacy}."))
        || arguments.contains(&format!("\"{forbidden}\""))
        || arguments.contains(&format!("\"{legacy}\""))
}

fn assert_memory_write_for_document(
    case: &ProjectSetupScenario,
    calls: &[(String, String)],
    document: &ExpectedDocument,
) {
    let matched = calls.iter().any(|(name, arguments)| {
        name == "builtin.memory_write"
            && arguments.contains(document.path)
            && document
                .fragments
                .iter()
                .all(|fragment| arguments.contains(fragment))
    });
    assert!(
        matched,
        "expected {} to write {} with fragments {:?}; recorded calls: {calls:#?}",
        case.fixture, document.path, document.fragments
    );
}

async fn replay_project_setup_scenario(case: &ProjectSetupScenario) {
    let mut trace = load_qa_trace(case.fixture);
    let http_exchanges = trace.http_exchanges.clone();
    strip_expected_tool_results(&mut trace);
    let gateway =
        RebornTraceReplayModelGateway::from_trace(trace).expect("replay gateway from fixture");

    let root = tempfile::tempdir().expect("tempdir");
    let runtime = build_qa_trace_runtime_with_http_exchanges(
        &root,
        Arc::new(gateway.clone()),
        http_exchanges,
    )
    .await;
    seed_qa_trace_world(&root, &runtime, &case.setup).await;

    let replies = send_qa_turns(&runtime, case.turns).await;
    assert_eq!(
        replies.len(),
        case.turns.len(),
        "replayed {} should produce one reply per turn",
        case.fixture
    );
    for reply in replies {
        assert!(
            reply.is_successful_final_reply(),
            "replayed {} should finalize successfully; status {:?}",
            case.fixture,
            reply.status
        );
    }
    gateway.assert_exhausted();

    for document in case.expected_documents {
        let content = read_qa_memory_document(&runtime, document.path).await;
        for fragment in document.fragments {
            assert!(
                content.contains(fragment),
                "replayed {} document {} should contain {fragment:?}; content: {content:?}",
                case.fixture,
                document.path
            );
        }
    }

    runtime.shutdown().await.expect("runtime shutdown");
}
