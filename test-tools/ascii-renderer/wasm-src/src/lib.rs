//! ASCII Renderer WASM tool for IronClaw (#5459 test fixture).
//!
//! The pure-compute case: an admin imports this tool, activates it, and any
//! user can ask the agent to draw ASCII art. It declares only the
//! `dispatch_capability` effect — NO network, NO credential — so it publishes
//! and completes with zero obligations.

mod types;

use types::{AsciiArt, DrawInput};

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../../wit/tool.wit",
});

struct AsciiRendererTool;

fn art_for(subject: &str) -> Option<&'static str> {
    match subject.trim().to_lowercase().as_str() {
        "cat" => Some(
            r#" /\_/\
( o.o )
 > ^ <"#,
        ),
        "dog" => Some(
            r#" / \__
(    @\___
 /         O
/    (_____/"#,
        ),
        "robot" | "" => Some(
            r#"  [ o_o ]
 /|_____|\
   |   |
  =d   b="#,
        ),
        _ => None,
    }
}

impl exports::near::agent::tool::Guest for AsciiRendererTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        crate::near::agent::host::log(
            crate::near::agent::host::LogLevel::Info,
            "ascii-renderer.draw: rendering canned ASCII art",
        );

        // Lenient parse: empty/blank params or an unexpected shape both fall
        // back to the default drawing (this is a toy tool).
        let params = req.params.trim();
        let input: DrawInput = if params.is_empty() {
            DrawInput::default()
        } else {
            serde_json::from_str(params).unwrap_or_default()
        };

        let requested = input.subject.unwrap_or_else(|| "robot".to_string());
        let (subject, art) = match art_for(&requested) {
            Some(art) => (requested, art),
            // Unknown subject -> default to robot rather than erroring.
            None => ("robot".to_string(), art_for("robot").unwrap_or("")),
        };

        let rendered = AsciiArt {
            subject,
            art: art.to_string(),
        };

        match serde_json::to_string(&rendered) {
            Ok(output) => exports::near::agent::tool::Response {
                output: Some(output),
                error: None,
            },
            Err(error) => exports::near::agent::tool::Response {
                output: None,
                error: Some(format!("failed to serialize ascii art: {error}")),
            },
        }
    }

    fn schema() -> String {
        // No enum on `subject`: the tool deliberately accepts any string and
        // falls back to robot, so the advertised contract must not promise a
        // closed set it doesn't enforce.
        r#"{"type":"object","properties":{"subject":{"type":"string","description":"cat, dog, or robot; unknown subjects fall back to robot"}},"additionalProperties":false}"#
            .to_string()
    }

    fn description() -> String {
        "Render a small piece of ASCII art. Optional `subject` selects the drawing \
         (cat, dog, or robot); defaults to robot. Use this whenever the user asks for \
         ASCII art or a quick drawing. No network access, no arguments required."
            .to_string()
    }
}

export!(AsciiRendererTool);
