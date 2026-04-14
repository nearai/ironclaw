//! Widget registry: loads widget configuration from workspace.
//!
//! In Phase 1, this is a simple factory that creates the built-in widgets.
//! Future phases will support loading custom widget manifests from
//! `tui/widgets/{id}/manifest.json` in the workspace.

use crate::layout::TuiLayout;

use super::TuiWidget;
use super::command_palette::CommandPaletteWidget;
use super::conversation::ConversationWidget;
use super::dashboard::DashboardWidget;
use super::header::HeaderWidget;
use super::help_overlay::HelpOverlayWidget;
use super::input_box::InputBoxWidget;
use super::logs::LogsWidget;
use super::model_picker::ModelPickerWidget;
use super::settings::SettingsWidget;
use super::status_bar::StatusBarWidget;
use super::tab_bar::TabBarWidget;
use super::thread_picker::ThreadPickerWidget;
use super::work_sidebar::WorkSidebarWidget;

/// Create the default set of built-in widgets.
pub fn create_default_widgets(layout: &TuiLayout) -> BuiltinWidgets {
    let theme = layout.resolve_theme();

    BuiltinWidgets {
        header: HeaderWidget::new(theme.clone()),
        tab_bar: TabBarWidget::new(theme.clone()),
        conversation: ConversationWidget::new(theme.clone()),
        dashboard: DashboardWidget::new(theme.clone()),
        logs: LogsWidget::new(theme.clone()),
        settings: SettingsWidget::new(theme.clone()),
        input_box: InputBoxWidget::new(theme.clone()),
        status_bar: StatusBarWidget::new(theme.clone()),
        approval: super::approval::ApprovalWidget::new(theme.clone()),
        help: HelpOverlayWidget::new(theme.clone()),
        thread_picker: ThreadPickerWidget::new(theme.clone()),
        model_picker: ModelPickerWidget::new(theme.clone()),
        work_sidebar: WorkSidebarWidget::new(theme.clone()),
        command_palette: CommandPaletteWidget::new(theme),
    }
}

/// Container for all built-in widgets.
///
/// We use concrete types instead of `Box<dyn TuiWidget>` so callers can
/// access widget-specific methods (e.g., `input_box.take_input()`).
pub struct BuiltinWidgets {
    pub header: HeaderWidget,
    pub tab_bar: TabBarWidget,
    pub conversation: ConversationWidget,
    pub dashboard: DashboardWidget,
    pub logs: LogsWidget,
    pub settings: SettingsWidget,
    pub input_box: InputBoxWidget,
    pub status_bar: StatusBarWidget,
    pub approval: super::approval::ApprovalWidget,
    pub help: HelpOverlayWidget,
    pub thread_picker: ThreadPickerWidget,
    pub model_picker: ModelPickerWidget,
    pub work_sidebar: WorkSidebarWidget,
    pub command_palette: CommandPaletteWidget,
}

/// Get references to all widgets as trait objects for generic iteration.
impl BuiltinWidgets {
    pub fn all(&self) -> Vec<&dyn TuiWidget> {
        vec![
            &self.header,
            &self.tab_bar,
            &self.conversation,
            &self.dashboard,
            &self.logs,
            &self.settings,
            &self.input_box,
            &self.status_bar,
            &self.approval,
            &self.help,
            &self.work_sidebar,
        ]
        // Note: command_palette is not included here because it renders
        // via a custom method (render_palette) rather than the TuiWidget trait.
    }
}
