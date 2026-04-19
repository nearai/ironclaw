//! Software builder for creating programs and tools using LLM-driven code generation.
//!
//! This module provides a general-purpose software building capability that:
//! - Uses an agent loop similar to Codex for iterative development
//! - Can build any software (binaries, libraries, scripts)
//! - Has special context injection when building WASM tools
//! - Integrates with existing tool loading infrastructure
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────────┐
//! │                          Software Build Loop                                 │
//! │                                                                              │
//! │  1. Analyze requirement ─▶ Determine project type, language, structure      │
//! │  2. Generate scaffold   ─▶ Create initial project files                     │
//! │  3. Implement code      ─▶ Write the actual implementation                  │
//! │  4. Build/compile       ─▶ Run build commands (cargo, npm, etc.)            │
//! │  5. Fix errors          ─▶ Parse errors, modify code, retry                 │
//! │  6. Test                ─▶ Run tests, fix failures                          │
//! │  7. Validate            ─▶ For WASM tools, verify interface compliance      │
//! │  8. Package             ─▶ Produce final artifact                           │
//! └─────────────────────────────────────────────────────────────────────────────┘
//! ```

mod core;
mod templates;
mod testing;
mod validation;

pub use core::{
    BuildLog, BuildPhase, BuildRequirement, BuildResult, BuildSoftwareTool, BuilderConfig,
    Language, LlmSoftwareBuilder, SoftwareBuilder, SoftwareType,
};
pub use templates::{Template, TemplateEngine, TemplateType};
pub use testing::{TestCase, TestHarness, TestResult, TestSuite};
pub use validation::{ValidationError, ValidationResult, WasmValidator};
