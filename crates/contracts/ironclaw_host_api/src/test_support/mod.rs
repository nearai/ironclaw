//! Feature-gated test-support helpers (`test-support`), never compiled into
//! production binaries. `src/test_support/**` is the repo-wide convention for
//! this seam (see `scripts/check_no_panics.py`'s `is_test_only_path`, and
//! e.g. `ironclaw_agent_loop`, `ironclaw_composition`) — panics here
//! are debugging aids for callers, not production code paths.

pub mod messaging_conformance;
