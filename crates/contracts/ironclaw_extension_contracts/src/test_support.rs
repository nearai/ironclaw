//! Feature-gated test-support fixtures for extension capability consumers (never compiled
//! into production binaries; see `scripts/check_no_panics.py` for the
//! canonical module convention).

pub mod conformance;
pub mod fakes;
