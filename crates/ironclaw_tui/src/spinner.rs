//! Configurable spinner definitions inspired by <https://github.com/sindresorhus/cli-spinners>.
//!
//! Each [`Spinner`] has an `interval_ms` (frame duration) and a `frames` slice.
//! The active frame is computed from elapsed time so spinners animate at their
//! natural speed regardless of the TUI tick rate.

/// A spinner animation definition.
#[derive(Debug, Clone, Copy)]
pub struct Spinner {
    /// Milliseconds per frame.
    pub interval_ms: u64,
    /// Animation frames cycled in order.
    pub frames: &'static [&'static str],
}

impl Spinner {
    /// Return the frame to display given the number of TUI ticks elapsed
    /// and the TUI tick interval in milliseconds.
    pub fn frame(&self, tick_count: usize, tick_ms: u64) -> &'static str {
        let elapsed_ms = tick_count as u64 * tick_ms;
        let idx = (elapsed_ms / self.interval_ms) as usize % self.frames.len();
        self.frames[idx]
    }
}

/// Named spinner variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpinnerKind {
    #[default]
    Dots,
    Dots2,
    Line,
    Arc,
    Star,
    Bounce,
    BouncingBar,
    Moon,
    Hamburger,
}

impl SpinnerKind {
    /// Get the [`Spinner`] definition for this variant.
    pub fn definition(self) -> Spinner {
        match self {
            Self::Dots => DOTS,
            Self::Dots2 => DOTS2,
            Self::Line => LINE,
            Self::Arc => ARC,
            Self::Star => STAR,
            Self::Bounce => BOUNCE,
            Self::BouncingBar => BOUNCING_BAR,
            Self::Moon => MOON,
            Self::Hamburger => HAMBURGER,
        }
    }

    /// All available spinner variants.
    pub const ALL: &'static [SpinnerKind] = &[
        Self::Dots,
        Self::Dots2,
        Self::Line,
        Self::Arc,
        Self::Star,
        Self::Bounce,
        Self::BouncingBar,
        Self::Moon,
        Self::Hamburger,
    ];

    /// Human-readable name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Dots => "dots",
            Self::Dots2 => "dots2",
            Self::Line => "line",
            Self::Arc => "arc",
            Self::Star => "star",
            Self::Bounce => "bounce",
            Self::BouncingBar => "bouncingBar",
            Self::Moon => "moon",
            Self::Hamburger => "hamburger",
        }
    }
}

// ── Spinner definitions ────────────────────────────────────────────────

pub const DOTS: Spinner = Spinner {
    interval_ms: 80,
    frames: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
};

pub const DOTS2: Spinner = Spinner {
    interval_ms: 80,
    frames: &["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"],
};

pub const LINE: Spinner = Spinner {
    interval_ms: 130,
    frames: &["-", "\\", "|", "/"],
};

pub const ARC: Spinner = Spinner {
    interval_ms: 100,
    frames: &["◜", "◠", "◝", "◞", "◡", "◟"],
};

pub const STAR: Spinner = Spinner {
    interval_ms: 70,
    frames: &["✶", "✸", "✹", "✺", "✹", "✷"],
};

pub const BOUNCE: Spinner = Spinner {
    interval_ms: 120,
    frames: &["⠁", "⠂", "⠄", "⠂"],
};

pub const BOUNCING_BAR: Spinner = Spinner {
    interval_ms: 80,
    frames: &[
        "[    ]", "[=   ]", "[==  ]", "[=== ]", "[ ===]", "[  ==]", "[   =]", "[    ]", "[   =]",
        "[  ==]", "[ ===]", "[====]", "[=== ]", "[==  ]", "[=   ]",
    ],
};

pub const MOON: Spinner = Spinner {
    interval_ms: 80,
    frames: &["🌑", "🌒", "🌓", "🌔", "🌕", "🌖", "🌗", "🌘"],
};

pub const HAMBURGER: Spinner = Spinner {
    interval_ms: 100,
    frames: &["☱", "☲", "☴"],
};
