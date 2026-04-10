//! IronClaw Gateway — frontend assets, layout configuration, and widget
//! extension system.
//!
//! This crate owns the complete frontend served by the IronClaw web gateway:
//!
//! - **Embedded assets** (`assets` module): HTML, JS, CSS, i18n files compiled
//!   into the binary for zero-dependency serving.
//! - **Layout configuration** (`layout` module): Branding, tab order, feature
//!   flags — customizable per-tenant via workspace.
//! - **Widget system** (`widget` module): Self-contained frontend components
//!   that plug into named slots in the UI.
//! - **Bundle assembly** (`bundle` module): Combines base assets with workspace
//!   customizations into the final served HTML.

pub mod assets;
mod bundle;
mod layout;
mod widget;

pub use bundle::{FrontendBundle, NONCE_PLACEHOLDER, ResolvedWidget, assemble_index};
pub use layout::{
    BrandingColors, BrandingConfig, ChatConfig, LayoutConfig, TabConfig, WidgetInstanceConfig,
    is_safe_widget_id,
};
pub use widget::{WidgetManifest, WidgetSlot, scope_css};

/// Errors from frontend operations.
#[derive(Debug, thiserror::Error)]
pub enum FrontendError {
    #[error("Layout configuration is invalid: {reason}")]
    InvalidLayout { reason: String },

    #[error("Widget '{id}' not found")]
    WidgetNotFound { id: String },

    #[error("Widget manifest is invalid: {reason}")]
    InvalidManifest { reason: String },
}
