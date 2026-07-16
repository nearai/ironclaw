//! Gate zone widget: renders the pending gate/auth prompt (headline, body,
//! and available options) directly above the composer. Only invoked by
//! `ui::render` while `state.pending_gate.is_some()`.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::{AppState, PendingGate};

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let Some(gate) = &state.pending_gate else {
        return;
    };
    let (headline, body, options) = describe(gate);

    let mut lines = vec![Line::styled(headline, Style::default().fg(Color::Yellow))];
    if !body.is_empty() {
        lines.push(Line::raw(body));
    }
    lines.push(Line::styled(options, Style::default().fg(Color::Gray)));

    let block = Block::default().borders(Borders::ALL).title("gate");
    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn describe(gate: &PendingGate) -> (String, String, String) {
    match gate {
        PendingGate::Approval {
            headline,
            body,
            allow_always,
            ..
        } => {
            let options = if *allow_always {
                "[a] allow  [A] allow always  [d] deny  [esc] cancel".to_string()
            } else {
                "[a] allow  [d] deny  [esc] cancel".to_string()
            };
            (headline.clone(), body.clone(), options)
        }
        PendingGate::Auth {
            headline,
            body,
            challenge_kind,
            authorization_url,
            ..
        } => {
            let mut body_text = body.clone();
            if let Some(url) = authorization_url {
                if !body_text.is_empty() {
                    body_text.push('\n');
                }
                body_text.push_str("open: ");
                body_text.push_str(url);
            }
            if let Some(kind) = challenge_kind {
                if !body_text.is_empty() {
                    body_text.push('\n');
                }
                body_text.push_str("kind: ");
                body_text.push_str(kind);
            }
            (headline.clone(), body_text, "[esc] cancel".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::ui::test_support::buffer_text;

    fn draw(state: &AppState) -> String {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(f, area, state);
            })
            .unwrap();
        buffer_text(terminal.backend().buffer())
    }

    #[test]
    fn gate_zone_renders_headline_and_options_when_pending() {
        let state = AppState::default()
            .set_pending_gate(Some(PendingGate::approval_stub("Allow write_file?")));
        let content = draw(&state);
        assert!(content.contains("Allow write_file?"));
        assert!(content.contains("allow") && content.contains("deny"));
        assert!(
            content.contains("[esc] cancel"),
            "Esc now resolves the gate server-side, not just a local dismiss"
        );
    }

    #[test]
    fn auth_gate_renders_authorization_url_and_cancel_hint() {
        let state = AppState::default().set_pending_gate(Some(PendingGate::Auth {
            turn_run_id: "run-stub".to_string(),
            gate_ref: "gate-stub".to_string(),
            headline: "Connect Gmail".to_string(),
            body: "Sign in to continue.".to_string(),
            challenge_kind: Some("oauth_url".to_string()),
            authorization_url: Some("https://example.com/oauth".to_string()),
        }));
        let content = draw(&state);
        assert!(content.contains("Connect Gmail"));
        assert!(content.contains("https://example.com/oauth"));
        assert!(content.contains("[esc] cancel"));
        assert!(!content.contains("[a] allow"));
    }

    #[test]
    fn nothing_renders_when_no_gate_pending() {
        let state = AppState::default();
        let content = draw(&state);
        assert!(!content.contains("gate"));
    }
}
