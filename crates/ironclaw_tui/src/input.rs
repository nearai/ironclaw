//! Key handling and command parsing for the TUI.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::widgets::LogLevelFilter;

/// Parsed user command from keyboard input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputAction {
    /// Submit the current input text to the agent.
    Submit,
    /// Quit the TUI.
    Quit,
    /// Toggle sidebar visibility.
    ToggleSidebar,
    /// Toggle between Conversation and Logs tabs.
    ToggleLogs,
    /// Scroll conversation up.
    ScrollUp,
    /// Scroll conversation down.
    ScrollDown,
    /// Cancel / interrupt current operation.
    Interrupt,
    /// Navigate approval dialog up.
    ApprovalUp,
    /// Navigate approval dialog down.
    ApprovalDown,
    /// Confirm approval selection.
    ApprovalConfirm,
    /// Cancel approval (deny).
    ApprovalCancel,
    /// Quick approve.
    QuickApprove,
    /// Quick always-approve.
    QuickAlways,
    /// Quick deny.
    QuickDeny,
    /// Navigate command palette up.
    PaletteUp,
    /// Navigate command palette down.
    PaletteDown,
    /// Select the highlighted command palette item.
    PaletteSelect,
    /// Close the command palette.
    PaletteClose,
    /// Navigate input history backward (older).
    HistoryUp,
    /// Navigate input history forward (newer).
    HistoryDown,
    /// Toggle search mode on/off.
    SearchToggle,
    /// Jump to next search match.
    SearchNext,
    /// Jump to previous search match.
    SearchPrev,
    /// Toggle help overlay (F1).
    ToggleHelp,
    /// Expand most recent tool output (Ctrl+E).
    ExpandTool,
    /// Set log level filter (1-5 in Logs tab).
    LogFilter(LogLevelFilter),
    /// Scroll tool detail modal up.
    ToolDetailScrollUp,
    /// Scroll tool detail modal down.
    ToolDetailScrollDown,
    /// Close the tool detail modal.
    ToolDetailClose,
    /// Paste image from system clipboard (Ctrl+V).
    ClipboardPaste,
    /// Navigate thread picker up.
    ThreadPickerUp,
    /// Navigate thread picker down.
    ThreadPickerDown,
    /// Select the highlighted thread.
    ThreadPickerSelect,
    /// Close the thread picker.
    ThreadPickerClose,
    /// No recognized action — pass to input box.
    Forward,
}

/// Map a key event to an action, considering active modal/context state.
#[allow(clippy::too_many_arguments)]
pub fn map_key(
    key: KeyEvent,
    approval_active: bool,
    palette_active: bool,
    search_active: bool,
    help_active: bool,
    tool_detail_active: bool,
    logs_active: bool,
    thread_picker_active: bool,
) -> InputAction {
    if thread_picker_active {
        return map_thread_picker_key(key);
    }

    if approval_active {
        return map_approval_key(key);
    }

    if help_active {
        return map_help_key(key);
    }

    if tool_detail_active {
        return map_tool_detail_key(key);
    }

    if search_active {
        return map_search_key(key);
    }

    if palette_active {
        return map_palette_key(key);
    }

    // Log level filter keys only in logs tab
    if logs_active && let Some(action) = map_log_filter_key(key) {
        return action;
    }

    match (key.code, key.modifiers) {
        (KeyCode::Enter, KeyModifiers::NONE) => InputAction::Submit,
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => InputAction::Quit,
        (KeyCode::Char('b'), KeyModifiers::CONTROL) => InputAction::ToggleSidebar,
        (KeyCode::Char('l'), KeyModifiers::CONTROL) => InputAction::ToggleLogs,
        (KeyCode::Char('f'), KeyModifiers::CONTROL) => InputAction::SearchToggle,
        (KeyCode::Char('e'), KeyModifiers::CONTROL) => InputAction::ExpandTool,
        (KeyCode::Char('v'), KeyModifiers::CONTROL) => InputAction::ClipboardPaste,
        (KeyCode::F(1), _) => InputAction::ToggleHelp,
        (KeyCode::Esc, _) => InputAction::Interrupt,
        (KeyCode::PageUp, _) => InputAction::ScrollUp,
        (KeyCode::PageDown, _) => InputAction::ScrollDown,
        // Ctrl+Up / Ctrl+Down for scroll
        (KeyCode::Up, KeyModifiers::CONTROL) => InputAction::ScrollUp,
        (KeyCode::Down, KeyModifiers::CONTROL) => InputAction::ScrollDown,
        // Ctrl+P / Ctrl+N for input history navigation
        (KeyCode::Char('p'), KeyModifiers::CONTROL) => InputAction::HistoryUp,
        (KeyCode::Char('n'), KeyModifiers::CONTROL) => InputAction::HistoryDown,
        _ => InputAction::Forward,
    }
}

/// Map key events when the help overlay is active.
fn map_help_key(key: KeyEvent) -> InputAction {
    match (key.code, key.modifiers) {
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => InputAction::Quit,
        (KeyCode::Esc, _) | (KeyCode::F(1), _) => InputAction::ToggleHelp,
        _ => InputAction::Forward,
    }
}

/// Map key events when the tool detail modal is active.
fn map_tool_detail_key(key: KeyEvent) -> InputAction {
    match (key.code, key.modifiers) {
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => InputAction::Quit,
        (KeyCode::Esc, _) => InputAction::ToolDetailClose,
        (KeyCode::PageUp, _) | (KeyCode::Up, _) => InputAction::ToolDetailScrollUp,
        (KeyCode::PageDown, _) | (KeyCode::Down, _) => InputAction::ToolDetailScrollDown,
        _ => InputAction::Forward,
    }
}

/// Map number keys to log level filters (only when logs tab is active).
fn map_log_filter_key(key: KeyEvent) -> Option<InputAction> {
    if key.modifiers != KeyModifiers::NONE {
        return None;
    }
    match key.code {
        KeyCode::Char('1') => Some(InputAction::LogFilter(LogLevelFilter::Error)),
        KeyCode::Char('2') => Some(InputAction::LogFilter(LogLevelFilter::Warn)),
        KeyCode::Char('3') => Some(InputAction::LogFilter(LogLevelFilter::Info)),
        KeyCode::Char('4') => Some(InputAction::LogFilter(LogLevelFilter::Debug)),
        KeyCode::Char('5') => Some(InputAction::LogFilter(LogLevelFilter::All)),
        _ => None,
    }
}

/// Map key events when the search bar is active.
fn map_search_key(key: KeyEvent) -> InputAction {
    match (key.code, key.modifiers) {
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => InputAction::Quit,
        (KeyCode::Esc, _) => InputAction::SearchToggle,
        (KeyCode::Enter, KeyModifiers::NONE) => InputAction::SearchNext,
        (KeyCode::Enter, KeyModifiers::SHIFT) => InputAction::SearchPrev,
        _ => InputAction::Forward,
    }
}

/// Map key events when the command palette is active.
fn map_palette_key(key: KeyEvent) -> InputAction {
    match key.code {
        KeyCode::Up => InputAction::PaletteUp,
        KeyCode::Down => InputAction::PaletteDown,
        KeyCode::Enter | KeyCode::Tab => InputAction::PaletteSelect,
        KeyCode::Esc => InputAction::PaletteClose,
        KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => InputAction::Quit,
        _ => InputAction::Forward,
    }
}

/// Map key events when the thread picker modal is active.
fn map_thread_picker_key(key: KeyEvent) -> InputAction {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => InputAction::ThreadPickerUp,
        KeyCode::Down | KeyCode::Char('j') => InputAction::ThreadPickerDown,
        KeyCode::Enter => InputAction::ThreadPickerSelect,
        KeyCode::Esc => InputAction::ThreadPickerClose,
        KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => InputAction::Quit,
        _ => InputAction::Forward,
    }
}

/// Map key events when the approval dialog is active.
fn map_approval_key(key: KeyEvent) -> InputAction {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => InputAction::ApprovalUp,
        KeyCode::Down | KeyCode::Char('j') => InputAction::ApprovalDown,
        KeyCode::Enter => InputAction::ApprovalConfirm,
        KeyCode::Esc => InputAction::ApprovalCancel,
        KeyCode::Char('y') | KeyCode::Char('Y') => InputAction::QuickApprove,
        KeyCode::Char('a') | KeyCode::Char('A') => InputAction::QuickAlways,
        KeyCode::Char('n') | KeyCode::Char('N') => InputAction::QuickDeny,
        _ => InputAction::Forward,
    }
}

/// Parse a slash command from user input text.
pub fn parse_slash_command(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    if trimmed.starts_with('/') {
        Some(trimmed)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map_default(key: KeyEvent) -> InputAction {
        map_key(key, false, false, false, false, false, false, false)
    }

    fn map_approval(key: KeyEvent) -> InputAction {
        map_key(key, true, false, false, false, false, false, false)
    }

    fn map_palette(key: KeyEvent) -> InputAction {
        map_key(key, false, true, false, false, false, false, false)
    }

    fn map_search(key: KeyEvent) -> InputAction {
        map_key(key, false, false, true, false, false, false, false)
    }

    fn map_logs(key: KeyEvent) -> InputAction {
        map_key(key, false, false, false, false, false, true, false)
    }

    fn map_help(key: KeyEvent) -> InputAction {
        map_key(key, false, false, false, true, false, false, false)
    }

    fn map_tool_detail(key: KeyEvent) -> InputAction {
        map_key(key, false, false, false, false, true, false, false)
    }

    fn map_thread_picker(key: KeyEvent) -> InputAction {
        map_key(key, false, false, false, false, false, false, true)
    }

    #[test]
    fn enter_submits() {
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(map_default(key), InputAction::Submit);
    }

    #[test]
    fn ctrl_c_quits() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(map_default(key), InputAction::Quit);
    }

    #[test]
    fn ctrl_b_toggles_sidebar() {
        let key = KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL);
        assert_eq!(map_default(key), InputAction::ToggleSidebar);
    }

    #[test]
    fn ctrl_l_toggles_logs() {
        let key = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL);
        assert_eq!(map_default(key), InputAction::ToggleLogs);
    }

    #[test]
    fn esc_interrupts() {
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(map_default(key), InputAction::Interrupt);
    }

    #[test]
    fn f1_toggles_help() {
        let key = KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE);
        assert_eq!(map_default(key), InputAction::ToggleHelp);
    }

    #[test]
    fn ctrl_e_expands_tool() {
        let key = KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL);
        assert_eq!(map_default(key), InputAction::ExpandTool);
    }

    #[test]
    fn approval_mode_y_approves() {
        let key = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
        assert_eq!(map_approval(key), InputAction::QuickApprove);
    }

    #[test]
    fn approval_mode_n_denies() {
        let key = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE);
        assert_eq!(map_approval(key), InputAction::QuickDeny);
    }

    #[test]
    fn palette_up_down() {
        let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(map_palette(up), InputAction::PaletteUp);
        let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(map_palette(down), InputAction::PaletteDown);
    }

    #[test]
    fn palette_enter_selects() {
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(map_palette(key), InputAction::PaletteSelect);
    }

    #[test]
    fn palette_tab_selects() {
        let key = KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(map_palette(key), InputAction::PaletteSelect);
    }

    #[test]
    fn palette_esc_closes() {
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(map_palette(key), InputAction::PaletteClose);
    }

    #[test]
    fn palette_typing_forwards() {
        let key = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE);
        assert_eq!(map_palette(key), InputAction::Forward);
    }

    #[test]
    fn ctrl_p_history_up() {
        let key = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL);
        assert_eq!(map_default(key), InputAction::HistoryUp);
    }

    #[test]
    fn ctrl_n_history_down() {
        let key = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL);
        assert_eq!(map_default(key), InputAction::HistoryDown);
    }

    #[test]
    fn history_keys_ignored_in_approval_mode() {
        let key_p = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL);
        assert_eq!(map_approval(key_p), InputAction::Forward);
    }

    #[test]
    fn history_keys_ignored_in_palette_mode() {
        let key_p = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL);
        assert_eq!(map_palette(key_p), InputAction::Forward);
    }

    #[test]
    fn ctrl_f_toggles_search() {
        let key = KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL);
        assert_eq!(map_default(key), InputAction::SearchToggle);
    }

    #[test]
    fn search_esc_closes() {
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(map_search(key), InputAction::SearchToggle);
    }

    #[test]
    fn search_enter_next() {
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(map_search(key), InputAction::SearchNext);
    }

    #[test]
    fn search_shift_enter_prev() {
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT);
        assert_eq!(map_search(key), InputAction::SearchPrev);
    }

    #[test]
    fn search_typing_forwards() {
        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        assert_eq!(map_search(key), InputAction::Forward);
    }

    #[test]
    fn search_ctrl_c_quits() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(map_search(key), InputAction::Quit);
    }

    #[test]
    fn log_filter_keys_in_logs_tab() {
        let key1 = KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE);
        assert_eq!(
            map_logs(key1),
            InputAction::LogFilter(LogLevelFilter::Error)
        );
        let key5 = KeyEvent::new(KeyCode::Char('5'), KeyModifiers::NONE);
        assert_eq!(map_logs(key5), InputAction::LogFilter(LogLevelFilter::All));
    }

    #[test]
    fn log_filter_keys_not_in_chat_tab() {
        let key1 = KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE);
        assert_eq!(map_default(key1), InputAction::Forward);
    }

    #[test]
    fn help_esc_closes() {
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(map_help(key), InputAction::ToggleHelp);
    }

    #[test]
    fn help_f1_closes() {
        let key = KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE);
        assert_eq!(map_help(key), InputAction::ToggleHelp);
    }

    #[test]
    fn tool_detail_esc_closes() {
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(map_tool_detail(key), InputAction::ToolDetailClose);
    }

    #[test]
    fn tool_detail_scroll() {
        let up = KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE);
        assert_eq!(map_tool_detail(up), InputAction::ToolDetailScrollUp);
        let down = KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE);
        assert_eq!(map_tool_detail(down), InputAction::ToolDetailScrollDown);
    }

    #[test]
    fn ctrl_v_clipboard_paste() {
        let key = KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL);
        assert_eq!(map_default(key), InputAction::ClipboardPaste);
    }

    #[test]
    fn thread_picker_up_down() {
        let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(map_thread_picker(up), InputAction::ThreadPickerUp);
        let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(map_thread_picker(down), InputAction::ThreadPickerDown);
    }

    #[test]
    fn thread_picker_jk_navigation() {
        let j = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(map_thread_picker(j), InputAction::ThreadPickerDown);
        let k = KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE);
        assert_eq!(map_thread_picker(k), InputAction::ThreadPickerUp);
    }

    #[test]
    fn thread_picker_enter_selects() {
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(map_thread_picker(key), InputAction::ThreadPickerSelect);
    }

    #[test]
    fn thread_picker_esc_closes() {
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(map_thread_picker(key), InputAction::ThreadPickerClose);
    }

    #[test]
    fn thread_picker_ctrl_c_quits() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(map_thread_picker(key), InputAction::Quit);
    }

    #[test]
    fn slash_command_detected() {
        assert_eq!(parse_slash_command("/help"), Some("/help"));
        assert_eq!(parse_slash_command("  /quit  "), Some("/quit"));
        assert_eq!(parse_slash_command("hello"), None);
    }
}
