//! Modal popups: threads (switch/create/delete), automations
//! (list/pause-resume/rename), provider (providers -> models -> confirmed).
//! `ui::render` draws whichever one is open inside a `Clear`-backed centered
//! popup. Small render tests cover stable visibility and presentation
//! contracts; reducer behavior remains in `app::automations_modal` and
//! `app::provider_modal`.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};

use crate::app::{AutomationsModalState, Modal, ProviderModalState, ThreadsModalState};

pub fn render(frame: &mut Frame, area: Rect, modal: &Modal) {
    match modal {
        Modal::Threads(state) => render_threads(frame, area, state),
        Modal::Automations(state) => render_automations(frame, area, state),
        Modal::Provider(state) => render_provider(frame, area, state),
    }
}

fn render_selectable_list<'a>(
    frame: &mut Frame,
    area: Rect,
    items: Vec<ListItem<'a>>,
    block: Block<'a>,
    selected: usize,
) {
    let selected = (!items.is_empty()).then(|| selected.min(items.len() - 1));
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    let mut state = ListState::default().with_selected(selected);
    frame.render_stateful_widget(list, area, &mut state);
}

/// The `+ new` row is pinned first, ahead of every listed thread — it is
/// not a real `ThreadSummary`, just a create-thread affordance rendered at
/// index 0 of the list. `app::threads_modal::ThreadsModalState::selected`
/// indexes this same rendered order (0 = "+ new", 1..=threads.len() =
/// `threads[selected - 1]`), so the highlight below lines up with what
/// `Enter`/`d` act on.
fn render_threads(frame: &mut Frame, area: Rect, modal: &ThreadsModalState) {
    let mut items = vec![ListItem::new("+ new")];
    for thread in &modal.threads {
        let label = thread
            .title
            .clone()
            .unwrap_or_else(|| thread.thread_id.clone());
        items.push(ListItem::new(label));
    }
    let title = if modal.pending_delete_confirm {
        "threads (press d again to delete)"
    } else {
        "threads"
    };
    render_selectable_list(
        frame,
        area,
        items,
        Block::default().borders(Borders::ALL).title(title),
        modal.selected,
    );
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
            let mut style = Style::default();
            if let Some(hold) = &automation.active_hold {
                label.push(' ');
                label.push_str(hold_badge_label(&hold.reason));
                style = style.fg(hold_badge_color(&hold.reason));
            }
            ListItem::new(label).style(style)
        })
        .collect();
    render_selectable_list(
        frame,
        area,
        items,
        Block::default().borders(Borders::ALL).title("automations"),
        modal.selected,
    );
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
                .map(|provider| {
                    let marker = if provider.active { "● " } else { "  " };
                    let label = format!("{marker}{} ({})", provider.id, provider.adapter);
                    let mut style = Style::default();
                    if provider.active {
                        style = style.fg(Color::Green);
                    }
                    ListItem::new(label).style(style)
                })
                .collect();
            render_selectable_list(
                frame,
                area,
                items,
                Block::default().borders(Borders::ALL).title("providers"),
                *selected,
            );
        }
        ProviderModalState::Models {
            provider_id,
            models,
            selected,
            ..
        } => {
            let items: Vec<ListItem> = models
                .iter()
                .map(|model| ListItem::new(model.clone()))
                .collect();
            render_selectable_list(
                frame,
                area,
                items,
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("models: {provider_id}")),
                *selected,
            );
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

    fn draw_modal_at(modal: &Modal, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(f, area, modal);
            })
            .unwrap();
        buffer_text(terminal.backend().buffer())
    }

    fn draw_modal(modal: &Modal) -> String {
        draw_modal_at(modal, 80, 24)
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
    fn threads_modal_scrolls_to_keep_selected_row_visible() {
        let modal = Modal::Threads(ThreadsModalState {
            threads: (0..8).map(|index| thread(&format!("t-{index}"))).collect(),
            selected: 8,
            pending_delete_confirm: false,
            loading: false,
        });

        let content = draw_modal_at(&modal, 32, 5);

        assert!(
            content.contains("t-7"),
            "selected row must be visible: {content:?}"
        );
        assert!(
            !content.contains("+ new"),
            "viewport must move off the first row"
        );
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
    fn provider_modal_scrolls_to_keep_selected_row_visible() {
        let modal = Modal::Provider(ProviderModalState::Providers {
            providers: (0..8)
                .map(|index| provider(&format!("p-{index}"), "adapter", false))
                .collect(),
            selected: 7,
            loading: false,
        });

        let content = draw_modal_at(&modal, 32, 5);

        assert!(
            content.contains("p-7"),
            "selected row must be visible: {content:?}"
        );
        assert!(
            !content.contains("p-0"),
            "viewport must move off the first row"
        );
    }

    #[test]
    fn provider_models_scroll_to_keep_selected_row_visible() {
        let modal = Modal::Provider(ProviderModalState::Models {
            provider_id: "provider".to_string(),
            adapter: "adapter".to_string(),
            base_url: None,
            models: (0..8).map(|index| format!("model-{index}")).collect(),
            selected: 7,
            loading: false,
        });

        let content = draw_modal_at(&modal, 32, 5);

        assert!(
            content.contains("model-7"),
            "selected row must be visible: {content:?}"
        );
        assert!(
            !content.contains("model-0"),
            "viewport must move off the first row"
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
    fn automations_modal_scrolls_to_keep_selected_row_visible() {
        let modal = Modal::Automations(AutomationsModalState {
            automations: (0..8)
                .map(|index| automation(&format!("a-{index}"), &format!("Job {index}"), "active"))
                .collect(),
            selected: 7,
            loading: false,
            renaming: None,
        });

        let content = draw_modal_at(&modal, 32, 5);

        assert!(
            content.contains("Job 7"),
            "selected row must be visible: {content:?}"
        );
        assert!(
            !content.contains("Job 0"),
            "viewport must move off the first row"
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
