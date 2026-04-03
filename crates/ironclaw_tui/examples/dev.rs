//! Standalone TUI dev harness — renders the full TUI with mock data.
//!
//! Usage:
//!   cargo run -p ironclaw_tui --example dev
//!
//! Hot-reload loop (recompiles + restarts on any source change):
//!   cargo watch -x 'run -p ironclaw_tui --example dev' -w crates/ironclaw_tui/src
//!
//! This compiles in ~5s instead of minutes because it skips the entire
//! ironclaw binary (database, LLM, WASM, Docker, etc.).

use std::time::Duration;

use ironclaw_tui::{SkillCategory, ToolCategory, TuiAppConfig, TuiEvent, TuiLayout, start_tui};

fn mock_tool_categories() -> Vec<ToolCategory> {
    vec![
        ToolCategory {
            name: "browser".into(),
            tools: vec![
                "back".into(),
                "click".into(),
                "navigate".into(),
                "screenshot".into(),
            ],
        },
        ToolCategory {
            name: "file".into(),
            tools: vec!["read".into(), "write".into(), "search".into()],
        },
        ToolCategory {
            name: "general".into(),
            tools: vec![
                "echo".into(),
                "github".into(),
                "gmail".into(),
                "http".into(),
                "json".into(),
                "time".into(),
            ],
        },
        ToolCategory {
            name: "memory".into(),
            tools: vec![
                "read".into(),
                "search".into(),
                "tree".into(),
                "write".into(),
            ],
        },
        ToolCategory {
            name: "routine".into(),
            tools: vec![
                "create".into(),
                "delete".into(),
                "list".into(),
                "update".into(),
            ],
        },
        ToolCategory {
            name: "secret".into(),
            tools: vec!["delete".into(), "list".into()],
        },
        ToolCategory {
            name: "shell".into(),
            tools: vec!["exec".into()],
        },
        ToolCategory {
            name: "skill".into(),
            tools: vec![
                "install".into(),
                "list".into(),
                "remove".into(),
                "search".into(),
            ],
        },
        ToolCategory {
            name: "tool".into(),
            tools: vec![
                "activate".into(),
                "auth".into(),
                "info".into(),
                "install".into(),
                "list".into(),
                "remove".into(),
                "search".into(),
                "upgrade".into(),
            ],
        },
        ToolCategory {
            name: "web".into(),
            tools: vec!["fetch".into()],
        },
    ]
}

fn mock_skill_categories() -> Vec<SkillCategory> {
    vec![
        SkillCategory {
            name: "apple".into(),
            skills: vec![
                "apple-notes".into(),
                "apple-reminders".into(),
                "findmy".into(),
            ],
        },
        SkillCategory {
            name: "creative".into(),
            skills: vec![
                "ascii-art".into(),
                "ascii-video".into(),
                "excalidraw".into(),
            ],
        },
        SkillCategory {
            name: "data-science".into(),
            skills: vec!["jupyter-live-kernel".into()],
        },
        SkillCategory {
            name: "github".into(),
            skills: vec![
                "codebase-inspection".into(),
                "github-auth".into(),
                "github-code-r...".into(),
            ],
        },
        SkillCategory {
            name: "media".into(),
            skills: vec!["gif-search".into(), "heartmula".into(), "songsee".into()],
        },
        SkillCategory {
            name: "productivity".into(),
            skills: vec![
                "google-workspace".into(),
                "linear".into(),
                "notion".into(),
                "ocr".into(),
            ],
        },
        SkillCategory {
            name: "research".into(),
            skills: vec!["arxiv".into(), "blogwatcher".into(), "domain-intel".into()],
        },
        SkillCategory {
            name: "software-dev".into(),
            skills: vec!["code-review".into(), "plan".into(), "remote-pr".into()],
        },
    ]
}

fn main() {
    let config = TuiAppConfig {
        version: "0.22.0-dev".into(),
        model: "gpt-5.4".into(),
        layout: TuiLayout::default(),
        context_window: 128_000,
        tools: mock_tool_categories(),
        skills: mock_skill_categories(),
        workspace_path: std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "~/projects/ironclaw".into()),
        memory_count: 42,
        identity_files: vec!["AGENTS.md".into(), "SOUL.md".into(), "USER.md".into()],
    };

    let handle = start_tui(config);
    let event_tx = handle.event_tx;
    let mut msg_rx = handle.msg_rx;

    // Spawn a thread that simulates agent responses to user input
    let sim_tx = event_tx.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build tokio runtime"); // safety: example binary, not library code

        rt.block_on(async move {
            // Simulate initial status events after a short delay
            tokio::time::sleep(Duration::from_millis(500)).await;
            let _ = sim_tx
                .send(TuiEvent::SandboxStatus {
                    docker_available: true,
                    running_containers: 0,
                    status: "ready".into(),
                })
                .await;
            let _ = sim_tx
                .send(TuiEvent::SecretsStatus {
                    count: 3,
                    vault_unlocked: true,
                })
                .await;

            // Echo user messages back as mock agent responses
            while let Some(user_msg) = msg_rx.recv().await {
                let msg = &user_msg.text;
                // Simulate thinking
                let _ = sim_tx
                    .send(TuiEvent::Thinking("Processing...".into()))
                    .await;
                tokio::time::sleep(Duration::from_millis(300)).await;

                // Simulate tool call
                let truncated: String = msg.chars().take(40).collect();
                let _ = sim_tx
                    .send(TuiEvent::ToolStarted {
                        name: "echo".into(),
                        detail: Some(format!("\"{truncated}\"")),
                        call_id: None,
                    })
                    .await;
                tokio::time::sleep(Duration::from_millis(200)).await;
                let _ = sim_tx
                    .send(TuiEvent::ToolCompleted {
                        name: "echo".into(),
                        success: true,
                        error: None,
                        call_id: None,
                    })
                    .await;
                let _ = sim_tx
                    .send(TuiEvent::ToolResult {
                        name: "echo".into(),
                        preview: msg.clone(),
                        call_id: None,
                    })
                    .await;

                // Simulate streaming response
                let _ = sim_tx.send(TuiEvent::Thinking(String::new())).await;
                let response = format!(
                    "You said: **{msg}**\n\nThis is a mock response from the dev harness. \
                     Edit `crates/ironclaw_tui/src/` and watch it reload.",
                );
                for chunk in response.as_bytes().chunks(20) {
                    let _ = sim_tx
                        .send(TuiEvent::StreamChunk(
                            String::from_utf8_lossy(chunk).to_string(),
                        ))
                        .await;
                    tokio::time::sleep(Duration::from_millis(30)).await;
                }
                let _ = sim_tx
                    .send(TuiEvent::Response {
                        content: response,
                        thread_id: None,
                    })
                    .await;

                // Simulate cost
                let _ = sim_tx
                    .send(TuiEvent::TurnCost {
                        input_tokens: 1200,
                        output_tokens: 340,
                        cost_usd: "$0.002".into(),
                    })
                    .await;

                // Suggestions
                let _ = sim_tx
                    .send(TuiEvent::Suggestions {
                        suggestions: vec![
                            "Tell me more".into(),
                            "Show available tools".into(),
                            "Search memory".into(),
                        ],
                    })
                    .await;
            }
        });
    });

    // Block main thread until TUI exits
    handle.join_handle.join().expect("TUI thread panicked"); // safety: example binary, not library code
}
