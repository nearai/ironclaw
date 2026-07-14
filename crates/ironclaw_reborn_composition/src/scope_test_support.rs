//! Generic `ResourceScope` test fixture shared by `factory.rs` and
//! `runtime.rs`'s test modules. Split out of `approval_test_support.rs`
//! (#5970 review, Fix 3), whose approval-flavored name had absorbed this
//! unrelated fixture and made it hard to find.

use ironclaw_host_api::{InvocationId, ResourceScope, UserId};

/// Local-default `ResourceScope` for `user` — shared by the `factory.rs` and
/// `runtime.rs` test modules.
pub(crate) fn test_scope(user: UserId) -> ResourceScope {
    ResourceScope::local_default(user, InvocationId::new()).expect("valid local scope") // safety: test-only fixture setup.
}
