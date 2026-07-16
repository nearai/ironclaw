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
use ratatui::style::{Modifier, Style};
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

fn render_automations(frame: &mut Frame, area: Rect, modal: &AutomationsModalState) {
    let items: Vec<ListItem> = modal
        .automations
        .iter()
        .enumerate()
        .map(|(idx, automation)| {
            let label = match (&modal.renaming, idx == modal.selected) {
                (Some(draft), true) => format!("{} > {draft}", automation.name),
                _ => format!("{} [{}]", automation.name, automation.state),
            };
            ListItem::new(label).style(selected_style(idx, modal.selected))
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
                    let label = format!("{} ({})", provider.id, provider.adapter);
                    ListItem::new(label).style(selected_style(idx, *selected))
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

    use super::*;
    use crate::client::ThreadSummary;
    use crate::ui::test_support::buffer_text;

    fn thread(id: &str) -> ThreadSummary {
        ThreadSummary {
            thread_id: id.to_string(),
            title: None,
            created_at: None,
            updated_at: None,
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
}
