use serde::{Deserialize, Serialize};

/// Optional input for `ascii-renderer.draw`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct DrawInput {
    /// Which drawing to render: `cat`, `dog`, or `robot`. Defaults to `robot`.
    pub subject: Option<String>,
}

/// A rendered piece of ASCII art.
#[derive(Debug, Serialize)]
pub struct AsciiArt {
    pub subject: String,
    pub art: String,
}
