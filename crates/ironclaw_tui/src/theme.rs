//! Color palette and Ratatui `Style` helpers.
//!
//! Mirrors the design tokens from `src/cli/fmt.rs` (emerald accent, dim, success,
//! error, warning) but expressed as Ratatui `Color` / `Style` values.

use ratatui::style::{Color, Modifier, Style};
use serde::{Deserialize, Serialize};

/// Emerald green brand color (true-color).
pub const EMERALD: Color = Color::Rgb(52, 211, 153);

/// Named color palette used by the TUI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    pub bg: ThemeColor,
    pub fg: ThemeColor,
    pub accent: ThemeColor,
    pub dim: ThemeColor,
    pub success: ThemeColor,
    pub warning: ThemeColor,
    pub error: ThemeColor,
    pub border: ThemeColor,
    pub header_bg: ThemeColor,
    pub status_bg: ThemeColor,
}

/// Serialisable color representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ThemeColor {
    Named(String),
    Rgb { r: u8, g: u8, b: u8 },
}

impl ThemeColor {
    pub fn to_color(&self) -> Color {
        match self {
            Self::Named(name) => match name.as_str() {
                "black" => Color::Black,
                "white" => Color::White,
                "red" => Color::Red,
                "green" => Color::Green,
                "yellow" => Color::Yellow,
                "blue" => Color::Blue,
                "magenta" => Color::Magenta,
                "cyan" => Color::Cyan,
                "gray" | "grey" => Color::Gray,
                "dark_gray" | "dark_grey" => Color::DarkGray,
                "reset" => Color::Reset,
                _ => Color::Reset,
            },
            Self::Rgb { r, g, b } => Color::Rgb(*r, *g, *b),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

impl Theme {
    /// Default dark theme matching IronClaw's CLI colors.
    pub fn dark() -> Self {
        Self {
            name: "dark".to_string(),
            bg: ThemeColor::Named("reset".to_string()),
            fg: ThemeColor::Named("white".to_string()),
            accent: ThemeColor::Rgb {
                r: 52,
                g: 211,
                b: 153,
            },
            dim: ThemeColor::Named("dark_gray".to_string()),
            success: ThemeColor::Named("green".to_string()),
            warning: ThemeColor::Named("yellow".to_string()),
            error: ThemeColor::Named("red".to_string()),
            border: ThemeColor::Named("dark_gray".to_string()),
            header_bg: ThemeColor::Named("reset".to_string()),
            status_bg: ThemeColor::Named("reset".to_string()),
        }
    }

    /// Light theme variant.
    pub fn light() -> Self {
        Self {
            name: "light".to_string(),
            bg: ThemeColor::Named("white".to_string()),
            fg: ThemeColor::Named("black".to_string()),
            accent: ThemeColor::Rgb {
                r: 16,
                g: 163,
                b: 127,
            },
            dim: ThemeColor::Named("gray".to_string()),
            success: ThemeColor::Named("green".to_string()),
            warning: ThemeColor::Named("yellow".to_string()),
            error: ThemeColor::Named("red".to_string()),
            border: ThemeColor::Named("gray".to_string()),
            header_bg: ThemeColor::Named("white".to_string()),
            status_bg: ThemeColor::Named("white".to_string()),
        }
    }

    // ── Style constructors ────────────────────────────────────────

    pub fn accent_style(&self) -> Style {
        Style::default().fg(self.accent.to_color())
    }

    pub fn dim_style(&self) -> Style {
        Style::default().fg(self.dim.to_color())
    }

    pub fn success_style(&self) -> Style {
        Style::default().fg(self.success.to_color())
    }

    pub fn warning_style(&self) -> Style {
        Style::default().fg(self.warning.to_color())
    }

    pub fn error_style(&self) -> Style {
        Style::default().fg(self.error.to_color())
    }

    pub fn bold_style(&self) -> Style {
        Style::default()
            .fg(self.fg.to_color())
            .add_modifier(Modifier::BOLD)
    }

    pub fn bold_accent_style(&self) -> Style {
        Style::default()
            .fg(self.accent.to_color())
            .add_modifier(Modifier::BOLD)
    }

    pub fn border_style(&self) -> Style {
        Style::default().fg(self.border.to_color())
    }

    pub fn header_style(&self) -> Style {
        Style::default()
            .bg(self.header_bg.to_color())
            .fg(self.fg.to_color())
    }

    pub fn status_style(&self) -> Style {
        Style::default()
            .bg(self.status_bg.to_color())
            .fg(self.dim.to_color())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_theme_has_emerald_accent() {
        let theme = Theme::dark();
        assert_eq!(theme.accent.to_color(), EMERALD);
    }

    #[test]
    fn light_theme_name() {
        let theme = Theme::light();
        assert_eq!(theme.name, "light");
    }

    #[test]
    fn theme_color_named_round_trips() {
        let c = ThemeColor::Named("green".to_string());
        assert_eq!(c.to_color(), Color::Green);
    }

    #[test]
    fn theme_color_rgb_round_trips() {
        let c = ThemeColor::Rgb {
            r: 10,
            g: 20,
            b: 30,
        };
        assert_eq!(c.to_color(), Color::Rgb(10, 20, 30));
    }

    #[test]
    fn theme_serialization_round_trip() {
        let theme = Theme::dark();
        let json = serde_json::to_string(&theme).expect("serialize");
        let back: Theme = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.name, "dark");
    }
}
