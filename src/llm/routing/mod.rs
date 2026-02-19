//! Smart model routing for automatic model selection.
//!
//! This module provides complexity-based routing to automatically select
//! the most appropriate model tier for each request.
//!
//! # Architecture
//!
//! ```text
//! User Message
//!      │
//!      ▼
//! ┌──────────────────┐
//! │ Pattern Overrides │  ← Fast-path for obvious cases
//! └────────┬─────────┘
//!          │ no match
//!          ▼
//! ┌──────────────────┐
//! │ Complexity Scorer │  ← 13-dimension analysis
//! └────────┬─────────┘
//!          │ score 0-100
//!          ▼
//! ┌──────────────────┐
//! │   Tier Mapping   │  ← Config maps tier → model
//! └────────┬─────────┘
//!          │
//!          ▼
//!     LLM Provider
//! ```
//!
//! # Tiers
//!
//! - **Flash** (0-15): Simple requests like greetings, quick lookups
//! - **Standard** (16-40): Writing, comparisons, defined tasks
//! - **Pro** (41-65): Multi-step analysis, code review
//! - **Frontier** (66+): Critical decisions, security audits
//!
//! # Usage
//!
//! ```rust,ignore
//! use ironclaw::llm::routing::{Router, RouterConfig};
//!
//! let router = Router::with_defaults();
//! let decision = router.route("What time is it?");
//!
//! println!("Model: {}", decision.model);
//! println!("Tier: {}", decision.tier);
//! ```

mod router;
mod scorer;

pub use router::{PatternOverride, Router, RouterConfig, RoutingDecision};
pub use scorer::{score_complexity, score_complexity_with_weights, ScoreBreakdown, ScorerWeights, Tier};
