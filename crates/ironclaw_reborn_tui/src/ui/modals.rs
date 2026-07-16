//! Modal popups: threads (switch/create/delete), automations
//! (list/pause-resume/rename), provider (providers -> models -> confirmed).
//! `ui::render` draws whichever one is open inside a `Clear`-backed centered
//! popup. Per the plan, only the threads modal gets a dedicated render test
//! here (small, non-brittle set); automations/provider rendering is still
//! implemented (so the real event loop has something to draw) but ships
//! without UI-tier goldens — their reducer behavior is already covered in
//! `app::automations_modal`/`app::provider_modal`.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem};

use crate::app::{AutomationsModalState, Modal, ProviderModalState, ThreadsModalState};

pub fn render(frame: &mut Frame, area: Rect, modal: &Modal) {
    match modal {
        Modal::Threads(state) => render_threads(frame, area, state),
        Modal::Automations(state) => render_automations(frame, area, state),
        Modal::Provider(state) => render_provider(frame, area, state),
    }
}

fn selected_style(idx: usize, selected: usize) -> Style {
    if idx == selected {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    }
}

/// The `+ new` row is pinned first, ahead of every listed thread — it is
/// not a real `ThreadSummary`, just a create-thread affordance rendered at
/// index 0 of the list. `app::threads_modal::ThreadsModalState::selected`
/// indexes this same rendered order (0 = "+ new", 1..=threads.len() =
/// `threads[selected - 1]`), so the highlight below lines up with what
/// `Enter`/`d` act on.
fn render_threads(frame: &mut Frame, area: Rect, modal: &ThreadsModalState) {
    let mut items = vec![ListItem::new("+ new").style(selected_style(0, modal.selected))];
    for (idx, thread) in modal.threads.iter().enumerate() {
        let label = thread
            .title
            .clone()
            .unwrap_or_else(|| thread.thread_id.clone());
        items.push(ListItem::new(label).style(selected_style(idx + 1, modal.selected)));
    }
    let title = if modal.pending_delete_confirm {
        "threads (press d again to delete)"
    } else {
        "threads"
    };
    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(list, area);
}

/// Maps a raw `RebornAutomationHoldReason` wire string (see
/// `client::AutomationActiveHold::reason`'s doc comment) to a short badge
/// label, mirroring the WebUI's `ACTIVE_HOLD_PRESENTATION` table
/// (`automations-presenters.ts`): approval/auth/other are gate-parked and
/// need the user to act, in_progress is merely informational.
fn hold_badge_label(reason: &str) -> &'static str {
    match reason {
        "approval" => "⚠ approval",
        "auth" => "⚠ auth",
        "in_progress" => "⏳ in progress",
        _ => "⚠ held",
    }
}

/// Tone for [`hold_badge_label`]: `in_progress` is informational (cyan),
/// every gate-parked reason (including unknown/`other`) is a warning
/// (yellow) — same warning/info split as `ACTIVE_HOLD_PRESENTATION`.
fn hold_badge_color(reason: &str) -> Color {
    match reason {
        "in_progress" => Color::Cyan,
        _ => Color::Yellow,
    }
}

fn render_automations(frame: &mut Frame, area: Rect, modal: &AutomationsModalState) {
    let items: Vec<ListItem> = modal
        .automations
        .iter()
        .enumerate()
        .map(|(idx, automation)| {
            let mut label = match (&modal.renaming, idx == modal.selected) {
                (Some(draft), true) => format!("{} > {draft}", automation.name),
                _ => format!("{} [{}]", automation.name, automation.state),
            };
            let mut style = selected_style(idx, modal.selected);
            if let Some(hold) = &automation.active_hold {
                label.push(' ');
                label.push_str(hold_badge_label(&hold.reason));
                style = style.fg(hold_badge_color(&hold.reason));
            }
            ListItem::new(label).style(style)
        })
        .collect();
    let list = List::new(items).block(Block::default().borders(Borders::ALL).title("automations"));
    frame.render_widget(list, area);
}

fn render_provider(frame: &mut Frame, area: Rect, modal: &ProviderModalState) {
    match modal {
        ProviderModalState::Providers {
            providers,
            selected,
            ..
        } => {
            let items: Vec<ListItem> = providers
                .iter()
                .enumerate()
                .map(|(idx, provider)| {
                    let marker = if provider.active { "● " } else { "  " };
                    let label = format!("{marker}{} ({})", provider.id, provider.adapter);
                    let mut style = selected_style(idx, *selected);
                    if provider.active {
                        style = style.fg(Color::Green);
                    }
                    ListItem::new(label).style(style)
                })
                .collect();
            let list =
                List::new(items).block(Block::default().borders(Borders::ALL).title("providers"));
            frame.render_widget(list, area);
        }
        ProviderModalState::Models {
            provider_id,
            models,
            selected,
            ..
        } => {
            let items: Vec<ListItem> = models
                .iter()
                .enumerate()
                .map(|(idx, model)| {
                    ListItem::new(model.clone()).style(selected_style(idx, *selected))
                })
                .collect();
            let list = List::new(items).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("models: {provider_id}")),
            );
            frame.render_widget(list, area);
        }
        ProviderModalState::Confirmed {
            provider_id,
            model,
            test_result,
        } => {
            let status = match test_result {
                Some(result) if result.ok => format!("connected ({})", result.message),
                Some(result) => format!("failed: {}", result.message),
                None => "testing…".to_string(),
            };
            let items = vec![
                ListItem::new(format!("{provider_id} / {model}")),
                ListItem::new(status),
            ];
            let list =
                List::new(items).block(Block::default().borders(Borders::ALL).title("provider"));
            frame.render_widget(list, area);
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use ironclaw_product_workflow::LlmProviderView;

    use super::*;
    use crate::client::automations::AutomationActiveHold;
    use crate::client::{AutomationSummary, ThreadSummary};
    use crate::ui::test_support::buffer_text;

    fn thread(id: &str) -> ThreadSummary {
        ThreadSummary {
            thread_id: id.to_string(),
            title: None,
            created_at: None,
            updated_at: None,
        }
    }

    fn automation(id: &str, name: &str, state: &str) -> AutomationSummary {
        AutomationSummary {
            automation_id: id.to_string(),
            name: name.to_string(),
            state: state.to_string(),
            next_run_at: None,
            last_run_at: None,
            last_status: None,
            is_active: state == "active",
            active_hold: None,
            recent_runs: Vec::new(),
        }
    }

    fn held_automation(id: &str, name: &str, state: &str, reason: &str) -> AutomationSummary {
        AutomationSummary {
            active_hold: Some(AutomationActiveHold {
                reason: reason.to_string(),
                since: None,
                elapsed_occurrences: None,
                elapsed_occurrences_capped: false,
            }),
            ..automation(id, name, state)
        }
    }

    fn provider(id: &str, adapter: &str, active: bool) -> LlmProviderView {
        LlmProviderView {
            id: id.to_string(),
            description: id.to_string(),
            adapter: adapter.to_string(),
            default_model: "default-model".to_string(),
            base_url: None,
            builtin: true,
            active,
            active_model: None,
            api_key_required: false,
            accepts_api_key: true,
            api_key_set: false,
            can_list_models: true,
        }
    }

    fn draw_modal(modal: &Modal) -> String {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(f, area, modal);
            })
            .unwrap();
        buffer_text(terminal.backend().buffer())
    }

    #[test]
    fn threads_modal_renders_new_pinned_first() {
        let modal = Modal::Threads(ThreadsModalState {
            threads: vec![thread("t-1"), thread("t-2")],
            selected: 0,
            pending_delete_confirm: false,
            loading: false,
        });
        let content = draw_modal(&modal);
        let new_pos = content.find("+ new").expect("+ new pinned entry present");
        let t1_pos = content.find("t-1").expect("thread present");
        assert!(new_pos < t1_pos, "+ new must render before thread rows");
    }

    #[test]
    fn threads_modal_pending_delete_confirm_changes_title() {
        let modal = Modal::Threads(ThreadsModalState {
            threads: vec![thread("t-1")],
            selected: 0,
            pending_delete_confirm: true,
            loading: false,
        });
        let content = draw_modal(&modal);
        assert!(content.contains("press d again to delete"));
    }

    #[test]
    fn provider_modal_marks_active_provider() {
        let modal = Modal::Provider(ProviderModalState::Providers {
            providers: vec![
                provider("openai", "open_ai_completions", false),
                provider("anthropic", "anthropic", true),
            ],
            selected: 0,
            loading: false,
        });
        let content = draw_modal(&modal);
        let openai_line = content
            .lines()
            .find(|line| line.contains("openai"))
            .expect("openai row rendered");
        let anthropic_line = content
            .lines()
            .find(|line| line.contains("anthropic"))
            .expect("anthropic row rendered");
        assert!(
            anthropic_line.contains('●'),
            "active provider row must carry the active marker: {anthropic_line:?}"
        );
        assert!(
            !openai_line.contains('●'),
            "inactive provider row must not carry the active marker: {openai_line:?}"
        );
    }

    #[test]
    fn automations_modal_renders_hold_badge_for_gate_parked_row() {
        let modal = Modal::Automations(AutomationsModalState {
            automations: vec![
                held_automation("a-1", "Daily digest", "active", "approval"),
                automation("a-2", "Weekly report", "active"),
            ],
            selected: 0,
            loading: false,
            renaming: None,
        });
        let content = draw_modal(&modal);
        let held_line = content
            .lines()
            .find(|line| line.contains("Daily digest"))
            .expect("held automation row rendered");
        let free_line = content
            .lines()
            .find(|line| line.contains("Weekly report"))
            .expect("non-held automation row rendered");
        assert!(
            held_line.contains("approval"),
            "gate-parked row must render its hold badge: {held_line:?}"
        );
        assert!(
            !free_line.contains("approval") && !free_line.contains("held"),
            "row without an active_hold must render no badge: {free_line:?}"
        );
    }

    #[test]
    fn automations_modal_renders_in_progress_hold_distinctly_from_approval() {
        let modal = Modal::Automations(AutomationsModalState {
            automations: vec![held_automation(
                "a-1",
                "Nightly sync",
                "active",
                "in_progress",
            )],
            selected: 0,
            loading: false,
            renaming: None,
        });
        let content = draw_modal(&modal);
        let line = content
            .lines()
            .find(|line| line.contains("Nightly sync"))
            .expect("held automation row rendered");
        assert!(
            line.contains("in progress"),
            "in_progress hold must render its own label, not the approval one: {line:?}"
        );
    }
}
