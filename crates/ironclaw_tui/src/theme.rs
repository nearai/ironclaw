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
    #[serde(default = "default_header_fg")]
    pub header_fg: ThemeColor,
    pub status_bg: ThemeColor,
    #[serde(default = "default_status_fg")]
    pub status_fg: ThemeColor,
    #[serde(default = "default_nav_bg")]
    pub nav_bg: ThemeColor,
    #[serde(default = "default_nav_fg")]
    pub nav_fg: ThemeColor,
    #[serde(default = "default_nav_active_fg")]
    pub nav_active_fg: ThemeColor,
    #[serde(default = "default_panel_bg")]
    pub panel_bg: ThemeColor,
    #[serde(default = "default_panel_alt_bg")]
    pub panel_alt_bg: ThemeColor,
    #[serde(default = "default_surface_header_bg")]
    pub surface_header_bg: ThemeColor,
    #[serde(default = "default_surface_header_fg")]
    pub surface_header_fg: ThemeColor,
    #[serde(default = "default_selected_bg")]
    pub selected_bg: ThemeColor,
    #[serde(default = "default_tab_bar_bg")]
    pub tab_bar_bg: ThemeColor,
    #[serde(default = "default_tab_active_bg")]
    pub tab_active_bg: ThemeColor,
    #[serde(default = "default_tab_active_fg")]
    pub tab_active_fg: ThemeColor,
    #[serde(default = "default_tab_inactive_fg")]
    pub tab_inactive_fg: ThemeColor,
    #[serde(default = "default_chrome_border")]
    pub chrome_border: ThemeColor,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThemeOverrides {
    pub accent: Option<ThemeColor>,
    pub border: Option<ThemeColor>,
    pub header_bg: Option<ThemeColor>,
    pub header_fg: Option<ThemeColor>,
    pub status_bg: Option<ThemeColor>,
    pub status_fg: Option<ThemeColor>,
    pub nav_bg: Option<ThemeColor>,
    pub nav_fg: Option<ThemeColor>,
    pub nav_active_fg: Option<ThemeColor>,
    pub panel_bg: Option<ThemeColor>,
    pub panel_alt_bg: Option<ThemeColor>,
    pub surface_header_bg: Option<ThemeColor>,
    pub surface_header_fg: Option<ThemeColor>,
    pub selected_bg: Option<ThemeColor>,
    pub tab_bar_bg: Option<ThemeColor>,
    pub tab_active_bg: Option<ThemeColor>,
    pub tab_active_fg: Option<ThemeColor>,
    pub tab_inactive_fg: Option<ThemeColor>,
    pub chrome_border: Option<ThemeColor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemePresetMeta {
    pub id: &'static str,
    pub name: &'static str,
    pub tagline: &'static str,
}

pub const THEME_PRESETS: &[ThemePresetMeta] = &[
    ThemePresetMeta {
        id: "dark",
        name: "Iron Dark",
        tagline: "Default emerald control room",
    },
    ThemePresetMeta {
        id: "graphite",
        name: "Graphite",
        tagline: "Muted chrome with softer contrast",
    },
    ThemePresetMeta {
        id: "midnight_emerald",
        name: "Midnight Emerald",
        tagline: "High-contrast green-on-night operators theme",
    },
    ThemePresetMeta {
        id: "amber_terminal",
        name: "Amber Terminal",
        tagline: "Warm amber command-center glow",
    },
    ThemePresetMeta {
        id: "ice",
        name: "Ice",
        tagline: "Cool cyan shell with crisp panels",
    },
    ThemePresetMeta {
        id: "light",
        name: "Paper Light",
        tagline: "Bright, cleaner daytime cockpit",
    },
];

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

fn default_header_fg() -> ThemeColor {
    ThemeColor::Named("white".to_string())
}

fn default_status_fg() -> ThemeColor {
    ThemeColor::Named("dark_gray".to_string())
}

fn default_nav_bg() -> ThemeColor {
    ThemeColor::Rgb {
        r: 13,
        g: 17,
        b: 23,
    }
}

fn default_nav_fg() -> ThemeColor {
    ThemeColor::Rgb {
        r: 125,
        g: 137,
        b: 149,
    }
}

fn default_nav_active_fg() -> ThemeColor {
    ThemeColor::Rgb {
        r: 207,
        g: 250,
        b: 238,
    }
}

fn default_panel_bg() -> ThemeColor {
    ThemeColor::Rgb {
        r: 15,
        g: 23,
        b: 32,
    }
}

fn default_panel_alt_bg() -> ThemeColor {
    ThemeColor::Rgb {
        r: 19,
        g: 29,
        b: 40,
    }
}

fn default_surface_header_bg() -> ThemeColor {
    ThemeColor::Rgb {
        r: 17,
        g: 26,
        b: 36,
    }
}

fn default_surface_header_fg() -> ThemeColor {
    ThemeColor::Named("white".to_string())
}

fn default_selected_bg() -> ThemeColor {
    ThemeColor::Rgb {
        r: 20,
        g: 44,
        b: 41,
    }
}

fn default_tab_bar_bg() -> ThemeColor {
    ThemeColor::Rgb { r: 9, g: 12, b: 17 }
}

fn default_tab_active_bg() -> ThemeColor {
    ThemeColor::Rgb {
        r: 17,
        g: 26,
        b: 36,
    }
}

fn default_tab_active_fg() -> ThemeColor {
    ThemeColor::Rgb {
        r: 207,
        g: 250,
        b: 238,
    }
}

fn default_tab_inactive_fg() -> ThemeColor {
    ThemeColor::Rgb {
        r: 108,
        g: 122,
        b: 136,
    }
}

fn default_chrome_border() -> ThemeColor {
    ThemeColor::Rgb {
        r: 36,
        g: 45,
        b: 56,
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

impl Theme {
    pub fn apply_overrides(mut self, overrides: &ThemeOverrides) -> Self {
        if let Some(color) = overrides.accent.clone() {
            self.accent = color;
        }
        if let Some(color) = overrides.border.clone() {
            self.border = color;
        }
        if let Some(color) = overrides.header_bg.clone() {
            self.header_bg = color;
        }
        if let Some(color) = overrides.header_fg.clone() {
            self.header_fg = color;
        }
        if let Some(color) = overrides.status_bg.clone() {
            self.status_bg = color;
        }
        if let Some(color) = overrides.status_fg.clone() {
            self.status_fg = color;
        }
        if let Some(color) = overrides.nav_bg.clone() {
            self.nav_bg = color;
        }
        if let Some(color) = overrides.nav_fg.clone() {
            self.nav_fg = color;
        }
        if let Some(color) = overrides.nav_active_fg.clone() {
            self.nav_active_fg = color;
        }
        if let Some(color) = overrides.panel_bg.clone() {
            self.panel_bg = color;
        }
        if let Some(color) = overrides.panel_alt_bg.clone() {
            self.panel_alt_bg = color;
        }
        if let Some(color) = overrides.surface_header_bg.clone() {
            self.surface_header_bg = color;
        }
        if let Some(color) = overrides.surface_header_fg.clone() {
            self.surface_header_fg = color;
        }
        if let Some(color) = overrides.selected_bg.clone() {
            self.selected_bg = color;
        }
        if let Some(color) = overrides.tab_bar_bg.clone() {
            self.tab_bar_bg = color;
        }
        if let Some(color) = overrides.tab_active_bg.clone() {
            self.tab_active_bg = color;
        }
        if let Some(color) = overrides.tab_active_fg.clone() {
            self.tab_active_fg = color;
        }
        if let Some(color) = overrides.tab_inactive_fg.clone() {
            self.tab_inactive_fg = color;
        }
        if let Some(color) = overrides.chrome_border.clone() {
            self.chrome_border = color;
        }
        self
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
            header_bg: ThemeColor::Rgb {
                r: 10,
                g: 14,
                b: 20,
            },
            header_fg: default_header_fg(),
            status_bg: ThemeColor::Rgb {
                r: 10,
                g: 14,
                b: 20,
            },
            status_fg: default_status_fg(),
            nav_bg: default_nav_bg(),
            nav_fg: default_nav_fg(),
            nav_active_fg: default_nav_active_fg(),
            panel_bg: default_panel_bg(),
            panel_alt_bg: default_panel_alt_bg(),
            surface_header_bg: default_surface_header_bg(),
            surface_header_fg: default_surface_header_fg(),
            selected_bg: default_selected_bg(),
            tab_bar_bg: default_tab_bar_bg(),
            tab_active_bg: default_tab_active_bg(),
            tab_active_fg: default_tab_active_fg(),
            tab_inactive_fg: default_tab_inactive_fg(),
            chrome_border: default_chrome_border(),
        }
    }

    pub fn from_name(name: &str) -> Self {
        match name {
            "light" => Self::light(),
            "graphite" => Self::graphite(),
            "midnight_emerald" => Self::midnight_emerald(),
            "amber_terminal" => Self::amber_terminal(),
            "ice" => Self::ice(),
            _ => Self::dark(),
        }
    }

    pub fn preset_catalog() -> &'static [ThemePresetMeta] {
        THEME_PRESETS
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
            header_bg: ThemeColor::Rgb {
                r: 243,
                g: 246,
                b: 249,
            },
            header_fg: ThemeColor::Named("black".to_string()),
            status_bg: ThemeColor::Rgb {
                r: 243,
                g: 246,
                b: 249,
            },
            status_fg: ThemeColor::Named("gray".to_string()),
            nav_bg: ThemeColor::Rgb {
                r: 244,
                g: 247,
                b: 250,
            },
            nav_fg: ThemeColor::Rgb {
                r: 92,
                g: 104,
                b: 116,
            },
            nav_active_fg: ThemeColor::Rgb { r: 9, g: 92, b: 72 },
            panel_bg: ThemeColor::Rgb {
                r: 255,
                g: 255,
                b: 255,
            },
            panel_alt_bg: ThemeColor::Rgb {
                r: 248,
                g: 250,
                b: 252,
            },
            surface_header_bg: ThemeColor::Rgb {
                r: 236,
                g: 241,
                b: 245,
            },
            surface_header_fg: ThemeColor::Named("black".to_string()),
            selected_bg: ThemeColor::Rgb {
                r: 220,
                g: 245,
                b: 235,
            },
            tab_bar_bg: ThemeColor::Rgb {
                r: 239,
                g: 243,
                b: 247,
            },
            tab_active_bg: ThemeColor::Rgb {
                r: 229,
                g: 235,
                b: 241,
            },
            tab_active_fg: ThemeColor::Rgb { r: 9, g: 92, b: 72 },
            tab_inactive_fg: ThemeColor::Rgb {
                r: 102,
                g: 113,
                b: 125,
            },
            chrome_border: ThemeColor::Rgb {
                r: 210,
                g: 218,
                b: 226,
            },
        }
    }

    pub fn graphite() -> Self {
        Self {
            name: "graphite".to_string(),
            bg: ThemeColor::Named("reset".to_string()),
            fg: ThemeColor::Rgb {
                r: 232,
                g: 236,
                b: 241,
            },
            accent: ThemeColor::Rgb {
                r: 124,
                g: 194,
                b: 255,
            },
            dim: ThemeColor::Rgb {
                r: 114,
                g: 123,
                b: 135,
            },
            success: ThemeColor::Rgb {
                r: 118,
                g: 211,
                b: 159,
            },
            warning: ThemeColor::Rgb {
                r: 245,
                g: 191,
                b: 96,
            },
            error: ThemeColor::Rgb {
                r: 255,
                g: 122,
                b: 122,
            },
            border: ThemeColor::Rgb {
                r: 69,
                g: 78,
                b: 90,
            },
            header_bg: ThemeColor::Rgb {
                r: 18,
                g: 22,
                b: 28,
            },
            header_fg: ThemeColor::Rgb {
                r: 236,
                g: 240,
                b: 244,
            },
            status_bg: ThemeColor::Rgb {
                r: 18,
                g: 22,
                b: 28,
            },
            status_fg: ThemeColor::Rgb {
                r: 135,
                g: 144,
                b: 154,
            },
            nav_bg: ThemeColor::Rgb {
                r: 20,
                g: 24,
                b: 31,
            },
            nav_fg: ThemeColor::Rgb {
                r: 131,
                g: 141,
                b: 154,
            },
            nav_active_fg: ThemeColor::Rgb {
                r: 232,
                g: 240,
                b: 252,
            },
            panel_bg: ThemeColor::Rgb {
                r: 24,
                g: 29,
                b: 37,
            },
            panel_alt_bg: ThemeColor::Rgb {
                r: 28,
                g: 34,
                b: 43,
            },
            surface_header_bg: ThemeColor::Rgb {
                r: 24,
                g: 30,
                b: 39,
            },
            surface_header_fg: ThemeColor::Rgb {
                r: 236,
                g: 240,
                b: 244,
            },
            selected_bg: ThemeColor::Rgb {
                r: 42,
                g: 53,
                b: 66,
            },
            tab_bar_bg: ThemeColor::Rgb {
                r: 16,
                g: 20,
                b: 26,
            },
            tab_active_bg: ThemeColor::Rgb {
                r: 32,
                g: 39,
                b: 49,
            },
            tab_active_fg: ThemeColor::Rgb {
                r: 232,
                g: 240,
                b: 252,
            },
            tab_inactive_fg: ThemeColor::Rgb {
                r: 124,
                g: 133,
                b: 145,
            },
            chrome_border: ThemeColor::Rgb {
                r: 61,
                g: 70,
                b: 82,
            },
        }
    }

    pub fn midnight_emerald() -> Self {
        Self {
            name: "midnight_emerald".to_string(),
            bg: ThemeColor::Named("reset".to_string()),
            fg: ThemeColor::Rgb {
                r: 229,
                g: 255,
                b: 246,
            },
            accent: ThemeColor::Rgb {
                r: 94,
                g: 234,
                b: 190,
            },
            dim: ThemeColor::Rgb {
                r: 98,
                g: 126,
                b: 118,
            },
            success: ThemeColor::Rgb {
                r: 94,
                g: 234,
                b: 190,
            },
            warning: ThemeColor::Rgb {
                r: 250,
                g: 204,
                b: 21,
            },
            error: ThemeColor::Rgb {
                r: 248,
                g: 113,
                b: 113,
            },
            border: ThemeColor::Rgb {
                r: 36,
                g: 73,
                b: 65,
            },
            header_bg: ThemeColor::Rgb { r: 5, g: 16, b: 16 },
            header_fg: ThemeColor::Rgb {
                r: 221,
                g: 255,
                b: 244,
            },
            status_bg: ThemeColor::Rgb { r: 5, g: 16, b: 16 },
            status_fg: ThemeColor::Rgb {
                r: 94,
                g: 126,
                b: 118,
            },
            nav_bg: ThemeColor::Rgb { r: 5, g: 18, b: 18 },
            nav_fg: ThemeColor::Rgb {
                r: 99,
                g: 139,
                b: 127,
            },
            nav_active_fg: ThemeColor::Rgb {
                r: 220,
                g: 255,
                b: 244,
            },
            panel_bg: ThemeColor::Rgb { r: 8, g: 24, b: 24 },
            panel_alt_bg: ThemeColor::Rgb {
                r: 10,
                g: 30,
                b: 29,
            },
            surface_header_bg: ThemeColor::Rgb { r: 9, g: 27, b: 26 },
            surface_header_fg: ThemeColor::Rgb {
                r: 220,
                g: 255,
                b: 244,
            },
            selected_bg: ThemeColor::Rgb {
                r: 16,
                g: 55,
                b: 47,
            },
            tab_bar_bg: ThemeColor::Rgb { r: 4, g: 13, b: 13 },
            tab_active_bg: ThemeColor::Rgb {
                r: 12,
                g: 35,
                b: 33,
            },
            tab_active_fg: ThemeColor::Rgb {
                r: 220,
                g: 255,
                b: 244,
            },
            tab_inactive_fg: ThemeColor::Rgb {
                r: 102,
                g: 134,
                b: 126,
            },
            chrome_border: ThemeColor::Rgb {
                r: 25,
                g: 62,
                b: 56,
            },
        }
    }

    pub fn amber_terminal() -> Self {
        Self {
            name: "amber_terminal".to_string(),
            bg: ThemeColor::Named("reset".to_string()),
            fg: ThemeColor::Rgb {
                r: 255,
                g: 237,
                b: 203,
            },
            accent: ThemeColor::Rgb {
                r: 251,
                g: 191,
                b: 36,
            },
            dim: ThemeColor::Rgb {
                r: 155,
                g: 124,
                b: 83,
            },
            success: ThemeColor::Rgb {
                r: 250,
                g: 204,
                b: 21,
            },
            warning: ThemeColor::Rgb {
                r: 251,
                g: 191,
                b: 36,
            },
            error: ThemeColor::Rgb {
                r: 248,
                g: 113,
                b: 113,
            },
            border: ThemeColor::Rgb {
                r: 99,
                g: 72,
                b: 35,
            },
            header_bg: ThemeColor::Rgb { r: 24, g: 14, b: 5 },
            header_fg: ThemeColor::Rgb {
                r: 255,
                g: 243,
                b: 214,
            },
            status_bg: ThemeColor::Rgb { r: 24, g: 14, b: 5 },
            status_fg: ThemeColor::Rgb {
                r: 171,
                g: 128,
                b: 77,
            },
            nav_bg: ThemeColor::Rgb { r: 26, g: 15, b: 5 },
            nav_fg: ThemeColor::Rgb {
                r: 188,
                g: 146,
                b: 92,
            },
            nav_active_fg: ThemeColor::Rgb {
                r: 255,
                g: 244,
                b: 220,
            },
            panel_bg: ThemeColor::Rgb { r: 32, g: 20, b: 8 },
            panel_alt_bg: ThemeColor::Rgb { r: 39, g: 24, b: 9 },
            surface_header_bg: ThemeColor::Rgb { r: 35, g: 22, b: 8 },
            surface_header_fg: ThemeColor::Rgb {
                r: 255,
                g: 244,
                b: 220,
            },
            selected_bg: ThemeColor::Rgb {
                r: 64,
                g: 42,
                b: 17,
            },
            tab_bar_bg: ThemeColor::Rgb { r: 21, g: 12, b: 4 },
            tab_active_bg: ThemeColor::Rgb {
                r: 43,
                g: 28,
                b: 11,
            },
            tab_active_fg: ThemeColor::Rgb {
                r: 255,
                g: 244,
                b: 220,
            },
            tab_inactive_fg: ThemeColor::Rgb {
                r: 180,
                g: 139,
                b: 89,
            },
            chrome_border: ThemeColor::Rgb {
                r: 97,
                g: 68,
                b: 29,
            },
        }
    }

    pub fn ice() -> Self {
        Self {
            name: "ice".to_string(),
            bg: ThemeColor::Named("reset".to_string()),
            fg: ThemeColor::Rgb {
                r: 231,
                g: 247,
                b: 255,
            },
            accent: ThemeColor::Rgb {
                r: 103,
                g: 232,
                b: 249,
            },
            dim: ThemeColor::Rgb {
                r: 110,
                g: 138,
                b: 149,
            },
            success: ThemeColor::Rgb {
                r: 45,
                g: 212,
                b: 191,
            },
            warning: ThemeColor::Rgb {
                r: 251,
                g: 191,
                b: 36,
            },
            error: ThemeColor::Rgb {
                r: 248,
                g: 113,
                b: 113,
            },
            border: ThemeColor::Rgb {
                r: 45,
                g: 70,
                b: 83,
            },
            header_bg: ThemeColor::Rgb { r: 8, g: 19, b: 28 },
            header_fg: ThemeColor::Rgb {
                r: 234,
                g: 248,
                b: 255,
            },
            status_bg: ThemeColor::Rgb { r: 8, g: 19, b: 28 },
            status_fg: ThemeColor::Rgb {
                r: 115,
                g: 141,
                b: 151,
            },
            nav_bg: ThemeColor::Rgb { r: 8, g: 21, b: 31 },
            nav_fg: ThemeColor::Rgb {
                r: 126,
                g: 159,
                b: 173,
            },
            nav_active_fg: ThemeColor::Rgb {
                r: 228,
                g: 249,
                b: 255,
            },
            panel_bg: ThemeColor::Rgb {
                r: 10,
                g: 26,
                b: 36,
            },
            panel_alt_bg: ThemeColor::Rgb {
                r: 13,
                g: 31,
                b: 42,
            },
            surface_header_bg: ThemeColor::Rgb {
                r: 12,
                g: 29,
                b: 40,
            },
            surface_header_fg: ThemeColor::Rgb {
                r: 228,
                g: 249,
                b: 255,
            },
            selected_bg: ThemeColor::Rgb {
                r: 19,
                g: 49,
                b: 63,
            },
            tab_bar_bg: ThemeColor::Rgb { r: 7, g: 16, b: 24 },
            tab_active_bg: ThemeColor::Rgb {
                r: 17,
                g: 40,
                b: 54,
            },
            tab_active_fg: ThemeColor::Rgb {
                r: 228,
                g: 249,
                b: 255,
            },
            tab_inactive_fg: ThemeColor::Rgb {
                r: 124,
                g: 154,
                b: 168,
            },
            chrome_border: ThemeColor::Rgb {
                r: 37,
                g: 63,
                b: 77,
            },
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

    pub fn chrome_border_style(&self) -> Style {
        Style::default().fg(self.chrome_border.to_color())
    }

    pub fn panel_style(&self) -> Style {
        Style::default()
            .bg(self.panel_bg.to_color())
            .fg(self.fg.to_color())
    }

    pub fn panel_alt_style(&self) -> Style {
        Style::default()
            .bg(self.panel_alt_bg.to_color())
            .fg(self.fg.to_color())
    }

    pub fn nav_style(&self) -> Style {
        Style::default()
            .bg(self.nav_bg.to_color())
            .fg(self.nav_fg.to_color())
    }

    pub fn selected_style(&self) -> Style {
        Style::default()
            .bg(self.selected_bg.to_color())
            .fg(self.nav_active_fg.to_color())
            .add_modifier(Modifier::BOLD)
    }

    pub fn header_style(&self) -> Style {
        Style::default()
            .bg(self.header_bg.to_color())
            .fg(self.header_fg.to_color())
    }

    pub fn status_style(&self) -> Style {
        Style::default()
            .bg(self.status_bg.to_color())
            .fg(self.status_fg.to_color())
    }

    pub fn surface_header_style(&self) -> Style {
        Style::default()
            .bg(self.surface_header_bg.to_color())
            .fg(self.surface_header_fg.to_color())
    }

    pub fn tab_bar_style(&self) -> Style {
        Style::default()
            .bg(self.tab_bar_bg.to_color())
            .fg(self.tab_inactive_fg.to_color())
    }

    pub fn tab_active_style(&self) -> Style {
        Style::default()
            .bg(self.tab_active_bg.to_color())
            .fg(self.tab_active_fg.to_color())
            .add_modifier(Modifier::BOLD)
    }

    pub fn tab_inactive_style(&self) -> Style {
        Style::default()
            .bg(self.tab_bar_bg.to_color())
            .fg(self.tab_inactive_fg.to_color())
    }

    // ── Syntax highlighting (computed, not serialized) ────────────

    /// Red keywords (Claude Code palette).
    pub fn syntax_keyword_style(&self) -> Style {
        Style::default()
            .fg(Color::Rgb(255, 123, 114))
            .add_modifier(Modifier::BOLD)
    }

    /// Purple function calls.
    pub fn syntax_function_style(&self) -> Style {
        Style::default().fg(Color::Rgb(210, 168, 255))
    }

    /// Blue type names.
    pub fn syntax_type_style(&self) -> Style {
        Style::default().fg(Color::Rgb(121, 192, 255))
    }

    /// Cyan string literals.
    pub fn syntax_string_style(&self) -> Style {
        Style::default().fg(Color::Rgb(165, 214, 255))
    }

    /// Blue number literals.
    pub fn syntax_number_style(&self) -> Style {
        Style::default().fg(Color::Rgb(121, 192, 255))
    }

    /// Dim gray comments.
    pub fn syntax_comment_style(&self) -> Style {
        Style::default().fg(Color::Rgb(72, 79, 88))
    }

    /// Dim gray line numbers.
    pub fn line_number_style(&self) -> Style {
        Style::default().fg(Color::Rgb(72, 79, 88))
    }

    /// Orange macro invocations.
    pub fn syntax_macro_style(&self) -> Style {
        Style::default().fg(Color::Rgb(255, 166, 87))
    }

    // ── Diff colors ──────────────────────────────────────────────

    /// Green text on dark green background for added lines.
    pub fn diff_add_style(&self) -> Style {
        Style::default()
            .fg(Color::Rgb(63, 185, 80))
            .bg(Color::Rgb(18, 38, 30))
    }

    /// Red text on dark red background for deleted lines.
    pub fn diff_del_style(&self) -> Style {
        Style::default()
            .fg(Color::Rgb(248, 81, 73))
            .bg(Color::Rgb(42, 18, 21))
    }

    /// Bold green `+` marker.
    pub fn diff_add_marker_style(&self) -> Style {
        Style::default()
            .fg(Color::Rgb(63, 185, 80))
            .add_modifier(Modifier::BOLD)
    }

    /// Bold red `-` marker.
    pub fn diff_del_marker_style(&self) -> Style {
        Style::default()
            .fg(Color::Rgb(248, 81, 73))
            .add_modifier(Modifier::BOLD)
    }

    /// Blue hunk header (`@@`).
    pub fn diff_hunk_style(&self) -> Style {
        Style::default().fg(Color::Rgb(88, 166, 255))
    }

    // ── Tool dots ────────────────────────────────────────────────

    /// Yellow dot for action tools (Write, Edit, Bash).
    pub fn tool_action_dot_style(&self) -> Style {
        Style::default().fg(Color::Rgb(227, 179, 65))
    }

    /// Blue dot for Read tools.
    pub fn tool_read_dot_style(&self) -> Style {
        Style::default().fg(Color::Rgb(88, 166, 255))
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
