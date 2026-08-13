use std::io::Write;

use serde::Serialize;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) enum OutputMode {
    #[default]
    Text,
    Json,
}

pub(crate) trait Renderable: Serialize {
    fn render_text_to(&self, w: &mut impl Write) -> std::io::Result<()>;
}

pub(crate) fn output(dto: &impl Renderable, mode: OutputMode) -> anyhow::Result<()> {
    match mode {
        OutputMode::Text => {
            dto.render_text_to(&mut std::io::stdout())?;
            Ok(())
        }
        OutputMode::Json => {
            println!("{}", serde_json::to_string_pretty(dto)?);
            Ok(())
        }
    }
}

pub(crate) fn terminal_safe_text(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests;
