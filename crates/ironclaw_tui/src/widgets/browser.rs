//! Skills / Extensions browser rendering used by the Settings widget.
//!
//! When the Settings section is `Skills` or `Extensions`, the right-hand
//! list pane renders an installed-items browser instead of the generic
//! settings entry list. The data comes from [`AppState::skill_items`] and
//! [`AppState::extension_items`], populated by the host process
//! (`src/channels/tui.rs`) from the live `SkillRegistry` /
//! `ExtensionManager`.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::render::truncate;
use crate::theme::Theme;

use super::{AppState, ExtensionBrowserItem, SkillBrowserItem};

/// Render the Skills browser list into the given settings list area.
///
/// The caller owns the outer block chrome; we fill `inner` only.
pub(super) fn render_skill_list(
    theme: &Theme,
    area: Rect,
    buf: &mut Buffer,
    state: &AppState,
    focused: bool,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.chrome_border_style())
        .style(theme.panel_style())
        .title(" Installed skills ");
    let inner = block.inner(area);
    block.render(area, buf);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let items = &state.skill_items;
    if items.is_empty() {
        Paragraph::new(Line::from(Span::styled(
            "No skills installed yet. Place a SKILL.md in ~/.ironclaw/skills/ or install from ClawHub.",
            theme.dim_style(),
        )))
        .style(theme.panel_style())
        .render(inner, buf);
        return;
    }

    let rows = inner.height as usize;
    let visible = rows.saturating_sub(2);
    let total = items.len();
    let scroll = state.settings.skill_scroll.min(total);
    let end = (scroll + visible).min(total);
    let selected = state.settings.skill_selected.min(total.saturating_sub(1));

    let name_width = (inner.width as usize / 3).clamp(10, 28);

    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                format!("{:<width$}", "Skill", width = name_width + 2),
                theme.dim_style(),
            ),
            Span::styled(format!("{:<9}", "Trust"), theme.dim_style()),
            Span::styled("Description", theme.dim_style()),
        ]),
        Line::from(""),
    ];
    for (visible_index, item) in items.iter().enumerate().take(end).skip(scroll) {
        let is_selected = visible_index == selected;
        lines.push(skill_row(theme, item, name_width, is_selected, focused));
    }
    if scroll + visible < total {
        lines.push(Line::from(Span::styled(
            format!("  \u{2026} {} more", total - (scroll + visible)),
            theme.dim_style(),
        )));
    }

    Paragraph::new(lines)
        .style(theme.panel_style())
        .render(inner, buf);
}

fn skill_row(
    theme: &Theme,
    item: &SkillBrowserItem,
    name_width: usize,
    selected: bool,
    focused: bool,
) -> Line<'static> {
    let marker = if selected { "\u{25B6}" } else { " " };
    let base_style = if selected && focused {
        theme.selected_style()
    } else if selected {
        theme.bold_accent_style()
    } else {
        theme.panel_style()
    };
    let trust_style = if item.trust.eq_ignore_ascii_case("trusted") {
        theme.success_style()
    } else {
        theme.accent_style()
    };

    let desc_width = 40usize;
    Line::from(vec![
        Span::styled(format!("{marker} "), base_style),
        Span::styled(
            format!(
                "{:<width$}",
                truncate(&item.name, name_width),
                width = name_width
            ),
            base_style.add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(format!("{:<8}", truncate(&item.trust, 8)), trust_style),
        Span::raw(" "),
        Span::styled(truncate(&item.description, desc_width), theme.dim_style()),
    ])
}

/// Render the Extensions browser list into the given settings list area.
pub(super) fn render_extension_list(
    theme: &Theme,
    area: Rect,
    buf: &mut Buffer,
    state: &AppState,
    focused: bool,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.chrome_border_style())
        .style(theme.panel_style())
        .title(" Installed extensions ");
    let inner = block.inner(area);
    block.render(area, buf);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let items = &state.extension_items;
    if items.is_empty() {
        Paragraph::new(Line::from(Span::styled(
            "No extensions installed yet. Install WASM tools or MCP servers from the registry.",
            theme.dim_style(),
        )))
        .style(theme.panel_style())
        .render(inner, buf);
        return;
    }

    let rows = inner.height as usize;
    let visible = rows.saturating_sub(2);
    let total = items.len();
    let scroll = state.settings.extension_scroll.min(total);
    let end = (scroll + visible).min(total);
    let selected = state
        .settings
        .extension_selected
        .min(total.saturating_sub(1));

    let name_width = (inner.width as usize / 3).clamp(10, 28);

    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                format!("{:<width$}", "Extension", width = name_width + 2),
                theme.dim_style(),
            ),
            Span::styled(format!("{:<14}", "Kind"), theme.dim_style()),
            Span::styled("Status", theme.dim_style()),
        ]),
        Line::from(""),
    ];
    for (visible_index, item) in items.iter().enumerate().take(end).skip(scroll) {
        let is_selected = visible_index == selected;
        lines.push(extension_row(theme, item, name_width, is_selected, focused));
    }
    if scroll + visible < total {
        lines.push(Line::from(Span::styled(
            format!("  \u{2026} {} more", total - (scroll + visible)),
            theme.dim_style(),
        )));
    }

    Paragraph::new(lines)
        .style(theme.panel_style())
        .render(inner, buf);
}

fn extension_row(
    theme: &Theme,
    item: &ExtensionBrowserItem,
    name_width: usize,
    selected: bool,
    focused: bool,
) -> Line<'static> {
    let marker = if selected { "\u{25B6}" } else { " " };
    let base_style = if selected && focused {
        theme.selected_style()
    } else if selected {
        theme.bold_accent_style()
    } else {
        theme.panel_style()
    };
    let status = if item.active {
        "active"
    } else if item.authenticated {
        "ready"
    } else {
        "idle"
    };
    let status_style = match status {
        "active" => theme.success_style(),
        "ready" => theme.accent_style(),
        _ => theme.dim_style(),
    };

    Line::from(vec![
        Span::styled(format!("{marker} "), base_style),
        Span::styled(
            format!(
                "{:<width$}",
                truncate(&item.name, name_width),
                width = name_width
            ),
            base_style.add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{:<13}", truncate(&item.kind, 13)),
            theme.dim_style(),
        ),
        Span::raw(" "),
        Span::styled(status.to_string(), status_style),
    ])
}

/// Render the detail panel for the currently selected skill.
pub(super) fn render_skill_detail(theme: &Theme, area: Rect, buf: &mut Buffer, state: &AppState) {
    render_panel(theme, area, buf, " Skill detail ", |lines| {
        let items = &state.skill_items;
        let Some(item) = items.get(state.settings.skill_selected) else {
            lines.push(Line::from(Span::styled(
                "Select a skill to see details.",
                theme.dim_style(),
            )));
            return;
        };
        lines.push(Line::from(vec![
            Span::styled(item.name.clone(), theme.bold_style()),
            Span::raw("  "),
            Span::styled(format!("v{}", item.version), theme.dim_style()),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            item.description.clone(),
            theme.panel_style(),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("Trust   ", theme.dim_style()),
            Span::styled(
                item.trust.clone(),
                if item.trust.eq_ignore_ascii_case("trusted") {
                    theme.success_style()
                } else {
                    theme.accent_style()
                },
            ),
        ]));
        if !item.keywords.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("Activates on ", theme.dim_style()),
                Span::styled(item.keywords.join(", "), theme.panel_style()),
            ]));
        }
        if let Some(hint) = &item.usage_hint {
            lines.push(Line::from(Span::styled(hint.clone(), theme.panel_style())));
        }
        if item.has_requirements {
            lines.push(Line::from(Span::styled(
                "Bundle includes requirements.txt",
                theme.dim_style(),
            )));
        }
        if item.has_scripts {
            lines.push(Line::from(Span::styled(
                "Bundle includes scripts/",
                theme.dim_style(),
            )));
        }
        if let Some(source) = &item.install_source_url {
            lines.push(Line::from(vec![
                Span::styled("Source  ", theme.dim_style()),
                Span::styled(source.clone(), theme.dim_style()),
            ]));
        }
    });
}

/// Render the detail panel for the currently selected extension.
pub(super) fn render_extension_detail(
    theme: &Theme,
    area: Rect,
    buf: &mut Buffer,
    state: &AppState,
) {
    render_panel(theme, area, buf, " Extension detail ", |lines| {
        let items = &state.extension_items;
        let Some(item) = items.get(state.settings.extension_selected) else {
            lines.push(Line::from(Span::styled(
                "Select an extension to see details.",
                theme.dim_style(),
            )));
            return;
        };
        lines.push(Line::from(vec![
            Span::styled(item.name.clone(), theme.bold_style()),
            Span::raw("  "),
            Span::styled(
                item.version
                    .clone()
                    .map(|v| format!("v{v}"))
                    .unwrap_or_default(),
                theme.dim_style(),
            ),
        ]));
        lines.push(Line::from(""));
        if let Some(desc) = &item.description {
            lines.push(Line::from(Span::styled(desc.clone(), theme.panel_style())));
            lines.push(Line::from(""));
        }
        lines.push(Line::from(vec![
            Span::styled("Kind     ", theme.dim_style()),
            Span::styled(item.kind.clone(), theme.accent_style()),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Active   ", theme.dim_style()),
            Span::styled(
                if item.active { "yes" } else { "no" }.to_string(),
                if item.active {
                    theme.success_style()
                } else {
                    theme.dim_style()
                },
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Auth     ", theme.dim_style()),
            Span::styled(
                if item.authenticated { "yes" } else { "no" }.to_string(),
                if item.authenticated {
                    theme.success_style()
                } else {
                    theme.dim_style()
                },
            ),
        ]));
        if !item.tools.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("Tools ({})", item.tools.len()),
                theme.dim_style(),
            )));
            for tool in item.tools.iter().take(6) {
                lines.push(Line::from(Span::styled(
                    format!("  {tool}"),
                    theme.panel_style(),
                )));
            }
            if item.tools.len() > 6 {
                lines.push(Line::from(Span::styled(
                    format!("  \u{2026} {} more", item.tools.len() - 6),
                    theme.dim_style(),
                )));
            }
        }
    });
}

fn render_panel<F>(theme: &Theme, area: Rect, buf: &mut Buffer, title: &str, fill: F)
where
    F: FnOnce(&mut Vec<Line<'static>>),
{
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.chrome_border_style())
        .style(theme.panel_alt_style())
        .title(Span::styled(
            title.to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    block.render(area, buf);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let mut lines: Vec<Line<'static>> = Vec::new();
    fill(&mut lines);
    Paragraph::new(lines)
        .style(theme.panel_alt_style())
        .render(inner, buf);
}

#[cfg(test)]
mod tests {
    use super::super::SettingsState;
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn make_state_with_skills(count: usize) -> AppState {
        let mut state = AppState::default();
        state.settings.visible_rows = 4;
        state.skill_items = (0..count)
            .map(|i| SkillBrowserItem::new(format!("skill-{i}"), format!("desc {i}")))
            .collect();
        state
    }

    fn make_state_with_extensions(count: usize) -> AppState {
        let mut state = AppState::default();
        state.settings.visible_rows = 4;
        state.extension_items = (0..count)
            .map(|i| ExtensionBrowserItem::new(format!("ext-{i}"), "wasm_tool"))
            .collect();
        state
    }

    #[test]
    fn move_skill_selection_clamps_to_last_item() {
        let mut s = SettingsState {
            visible_rows: 3,
            ..Default::default()
        };
        s.move_skill_selection(10, 5);
        assert_eq!(s.skill_selected, 4);
    }

    #[test]
    fn move_skill_selection_clamps_above_zero() {
        let mut s = SettingsState {
            visible_rows: 3,
            skill_selected: 2,
            ..Default::default()
        };
        s.move_skill_selection(-10, 5);
        assert_eq!(s.skill_selected, 0);
    }

    #[test]
    fn move_skill_selection_no_panic_on_empty() {
        let mut s = SettingsState {
            visible_rows: 3,
            ..Default::default()
        };
        s.move_skill_selection(1, 0);
        assert_eq!(s.skill_selected, 0);
        assert_eq!(s.skill_scroll, 0);
    }

    #[test]
    fn page_skills_scrolls_by_visible_rows() {
        let mut s = SettingsState {
            visible_rows: 4,
            ..Default::default()
        };
        s.page_skills(1, 20);
        assert_eq!(s.skill_selected, 4);
    }

    #[test]
    fn move_extension_selection_tracks_scroll_offset() {
        let mut s = SettingsState {
            visible_rows: 3,
            ..Default::default()
        };
        // Move past the visible window to force scroll.
        for _ in 0..5 {
            s.move_extension_selection(1, 10);
        }
        assert_eq!(s.extension_selected, 5);
        assert!(s.extension_scroll > 0);
        assert!(s.extension_scroll <= s.extension_selected);
    }

    #[test]
    fn sync_skill_browser_clamps_when_list_shrinks() {
        let mut s = SettingsState {
            visible_rows: 3,
            skill_selected: 7,
            skill_scroll: 5,
            ..Default::default()
        };
        s.sync_skill_browser(3);
        assert_eq!(s.skill_selected, 2);
        assert_eq!(s.skill_scroll, 0);
    }

    #[test]
    fn render_skill_list_shows_empty_state_when_no_items() {
        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = AppState::default();
        let theme = Theme::from_name("dark");
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_skill_list(&theme, area, frame.buffer_mut(), &state, true);
            })
            .unwrap();
        let out = terminal.backend().buffer().clone();
        let contents = out.content().iter().map(|c| c.symbol()).collect::<String>();
        assert!(
            contents.contains("No skills installed"),
            "expected empty state, got: {contents}"
        );
    }

    #[test]
    fn render_skill_list_shows_names_and_highlights_selection() {
        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = make_state_with_skills(3);
        state.settings.skill_selected = 1;
        let theme = Theme::from_name("dark");
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_skill_list(&theme, area, frame.buffer_mut(), &state, true);
            })
            .unwrap();
        let out = terminal.backend().buffer().clone();
        let contents = out.content().iter().map(|c| c.symbol()).collect::<String>();
        assert!(contents.contains("skill-0"));
        assert!(contents.contains("skill-1"));
        assert!(contents.contains("skill-2"));
        // Selection marker on the chosen row.
        assert!(contents.contains("\u{25B6}"));
    }

    #[test]
    fn render_extension_list_shows_kind_and_status() {
        let backend = TestBackend::new(70, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = make_state_with_extensions(2);
        state.extension_items[0].active = true;
        state.extension_items[1].authenticated = true;
        let theme = Theme::from_name("dark");
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_extension_list(&theme, area, frame.buffer_mut(), &state, true);
            })
            .unwrap();
        let out = terminal.backend().buffer().clone();
        let contents = out.content().iter().map(|c| c.symbol()).collect::<String>();
        assert!(contents.contains("ext-0"));
        assert!(contents.contains("ext-1"));
        assert!(contents.contains("wasm_tool"));
        assert!(contents.contains("active"));
        assert!(contents.contains("ready"));
    }

    #[test]
    fn render_skill_detail_shows_trusted_badge_and_keywords() {
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = AppState::default();
        state.skill_items.push(SkillBrowserItem {
            name: "focus-mode".to_string(),
            version: "1.2.3".to_string(),
            description: "Helps with deep work".to_string(),
            trust: "Trusted".to_string(),
            keywords: vec!["focus".to_string(), "deep-work".to_string()],
            usage_hint: Some("Type /focus-mode".to_string()),
            has_requirements: true,
            has_scripts: false,
            install_source_url: None,
        });
        let theme = Theme::from_name("dark");
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_skill_detail(&theme, area, frame.buffer_mut(), &state);
            })
            .unwrap();
        let out = terminal.backend().buffer().clone();
        let contents = out.content().iter().map(|c| c.symbol()).collect::<String>();
        assert!(contents.contains("focus-mode"));
        assert!(contents.contains("v1.2.3"));
        assert!(contents.contains("Trusted"));
        assert!(contents.contains("focus"));
        assert!(contents.contains("deep-work"));
        assert!(contents.contains("requirements.txt"));
    }
}
