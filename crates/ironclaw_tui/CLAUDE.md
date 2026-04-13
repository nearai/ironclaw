# ironclaw_tui — Module Spec

## Overview

Ratatui-based terminal UI for IronClaw. Self-contained crate that provides:
- Widget system (`TuiWidget` trait) with built-in widgets (header, conversation, input, status bar, tool panel, thread list, approval modal)
- Layout engine with user-configurable JSON (`tui/layout.json` in workspace)
- Theme system (dark/light, custom colors)
- Event loop with crossterm input polling + external event merging

## Dependencies

- No dependency on the main `ironclaw` crate (avoids circular dependency)
- Channel trait bridge lives in `src/channels/tui.rs` in the main crate

## Communication

```
Main Crate (TuiChannel)          ironclaw_tui (TuiApp)
─────────────────────             ───────────────────
event_tx: Sender<TuiEvent> ────→ event_rx: renders UI
msg_rx: Receiver<String>  ←──── msg_tx: user input
```

## Key Bindings

| Key      | Action               |
|----------|----------------------|
| Enter    | Submit input         |
| Ctrl-C   | Quit                 |
| Ctrl-B   | Toggle sidebar       |
| Esc      | Interrupt/cancel     |
| PgUp/Dn  | Scroll conversation  |
| y/n/a    | Approval shortcuts   |

## Adding a Widget

1. Create `src/widgets/my_widget.rs`
2. Implement `TuiWidget` trait
3. Add to `BuiltinWidgets` in `registry.rs`
4. Wire into `render_frame()` in `app.rs`
