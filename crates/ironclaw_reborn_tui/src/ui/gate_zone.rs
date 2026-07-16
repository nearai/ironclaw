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
            token_input,
            ..
        } => {
            // Token-input sub-mode takes over the whole zone: a masked
            // echo of what's been typed plus its own hint line, in place of
            // the normal body/options. Mirrors `app::gate::dispatch_gate_key`
            // routing every key to `dispatch_token_input_key` while active.
            if let Some(buf) = token_input {
                let masked = "*".repeat(buf.chars().count());
                let mut body_text = body.clone();
                if !body_text.is_empty() {
                    body_text.push('\n');
                }
                body_text.push_str("token: ");
                body_text.push_str(&masked);
                return (
                    headline.clone(),
                    body_text,
                    "[enter] submit  [esc] cancel".to_string(),
                );
            }

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
            // `[o] open` only applies when there's a URL to open (mirrors
            // `app::gate::dispatch_gate_key`'s `o` handler, which is a no-op
            // without `authorization_url`). `[t] enter token` always
            // applies — the sub-mode itself is what actually submits.
            let options = if authorization_url.is_some() {
                "[o] open  [t] enter token  [esc] cancel".to_string()
            } else {
                "[t] enter token  [esc] cancel".to_string()
            };
            (headline.clone(), body_text, options)
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

    /// `PendingGate::Auth` fixture — a plain helper fn rather than a base
    /// value tests apply `..` functional-update syntax to: enum struct
    /// variants don't support `..base` (E0436, "requires a struct"), only
    /// plain structs do. Each test passes only the fields it varies;
    /// everything else takes the same default every other test uses.
    fn auth_gate(
        authorization_url: Option<&str>,
        challenge_kind: Option<&str>,
        token_input: Option<&str>,
    ) -> PendingGate {
        PendingGate::Auth {
            turn_run_id: "run-stub".to_string(),
            gate_ref: "gate-stub".to_string(),
            headline: "Connect Gmail".to_string(),
            body: "Sign in to continue.".to_string(),
            challenge_kind: challenge_kind.map(str::to_string),
            authorization_url: authorization_url.map(str::to_string),
            provider: None,
            account_label: None,
            token_input: token_input.map(str::to_string),
        }
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
    fn auth_gate_with_url_renders_open_and_cancel_hints() {
        let state = AppState::default().set_pending_gate(Some(auth_gate(
            Some("https://example.com/oauth"),
            Some("oauth_url"),
            None,
        )));
        let content = draw(&state);
        assert!(content.contains("Connect Gmail"));
        assert!(content.contains("https://example.com/oauth"));
        assert!(content.contains("[o] open"));
        assert!(content.contains("[t] enter token"));
        assert!(content.contains("[esc] cancel"));
        assert!(!content.contains("[a] allow"));
    }

    #[test]
    fn auth_gate_without_url_omits_open_hint_but_keeps_token_hint() {
        let state =
            AppState::default().set_pending_gate(Some(auth_gate(None, Some("manual_token"), None)));
        let content = draw(&state);
        assert!(!content.contains("[o] open"));
        assert!(content.contains("[t] enter token"));
        assert!(content.contains("[esc] cancel"));
    }

    #[test]
    fn token_input_sub_mode_renders_masked_buffer_and_submit_cancel_hints() {
        let state = AppState::default().set_pending_gate(Some(auth_gate(
            None,
            Some("manual_token"),
            Some("sekret"),
        )));
        let content = draw(&state);
        assert!(
            content.contains("******"),
            "typed token must render masked, not in the clear: {content}"
        );
        assert!(!content.contains("sekret"), "raw token must never render");
        assert!(content.contains("[enter] submit"));
        assert!(content.contains("[esc] cancel"));
        assert!(
            !content.contains("[a] allow") && !content.contains("[o] open"),
            "sub-mode replaces the normal gate options entirely"
        );
    }

    #[test]
    fn token_input_sub_mode_with_empty_buffer_renders_no_mask_characters() {
        let state = AppState::default().set_pending_gate(Some(auth_gate(
            None,
            Some("manual_token"),
            Some(""),
        )));
        let content = draw(&state);
        assert!(content.contains("token: "));
        assert!(content.contains("[enter] submit"));
    }

    #[test]
    fn nothing_renders_when_no_gate_pending() {
        let state = AppState::default();
        let content = draw(&state);
        assert!(!content.contains("gate"));
    }
}
