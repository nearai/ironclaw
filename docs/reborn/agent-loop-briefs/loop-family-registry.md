# WS-3.5 — Loop Family Registry

**Workstream:** WS-3.5 (parallel with WS-1, WS-2, WS-3 — depends only on WS-0)
**Crates touched:** `ironclaw_agent_loop` (NEW types), `ironclaw_reborn` (composition root)
**Depends on:** WS-0 (`LoopExecutionState`)
**Parallel with:** WS-1, WS-2, WS-3
**Master doc:** [`../agent-loop-skeleton.md`](../agent-loop-skeleton.md) §4, §4.5, §9, §11

---

## 1. Scope

Land the top-layer abstraction that profile resolution targets: `LoopFamily` as a first-class, Builtin-only, opaque bundle of (`LoopFamilyId`, `ComponentIdentity`, planner). The registry is a Guice-style singleton constructed by the `ironclaw_reborn` composition root; TurnRunner selection/plumbing is deferred until the planned-driver and run-profile selection workstreams land. Strategy traits stay sealed inside `ironclaw_agent_loop` — extensions never compose strategies; they extend via hooks (see master doc §9 and §9.1).

This brief establishes:

- `LoopFamilyId` — string-shaped newtype with associated consts for known ids.
- `ComponentIdentity` — content-addressed versioning primitive carried in checkpoint payload metadata and `LoopFamily.version`. Subsumes WS-4's `PlannerId`.
- `LoopFamily` — opaque type, `pub(crate)` constructor; holds the planner and identifies the family.
- `LoopFamilyRegistry` — singleton, built once by `ironclaw_reborn::app_loop_family::build_loop_family_registry()` from framework-provided family factories, immutable thereafter.
- `families::default` factory — the one production family in this skeleton.

## 2. Files

### NEW
- `crates/ironclaw_agent_loop/src/family.rs` — `LoopFamilyId`, `ComponentIdentity`, `LoopFamily`, `LoopFamilyRegistry`.
- `crates/ironclaw_agent_loop/src/families/mod.rs` — `pub fn default() -> LoopFamily`. Future siblings live here.

### EXTEND
- `crates/ironclaw_agent_loop/src/lib.rs` — export `family`, `families`. Keep strategy modules `pub(crate)`.
- `crates/ironclaw_reborn/src/app_loop_family.rs` (NEW) — `pub fn build_loop_family_registry() -> Result<Arc<LoopFamilyRegistry>, LoopFamilyRegistryError>` calling `LoopFamilyRegistry::with_families(...)`. This is the composition root: it's the one place that knows which families exist.

### NOT TOUCHED in this brief
- Strategy trait visibilities — WS-1/2/3 land them as `pub(crate)`; this brief just consumes them.
- `PlannedDriver` generic-collapse — WS-7 owns that.
- `TurnRunner` family resolution/plumbing — WS-7/WS-14 own the runner-facing path once `PlannedDriver` and run-profile selection exist.
- Master doc — separate amendment lands the cross-references.

## 3. Specification

### 3.1 `LoopFamilyId`

```rust
//! crates/ironclaw_agent_loop/src/family.rs

use std::borrow::Cow;

/// Identity for a Builtin loop family. String-shaped newtype: associated
/// consts name well-known ids; the type is open so future Builtin families
/// can add their own const without touching an enum.
///
/// Profile JSON serializes as `"default"`, `"coding"`, etc. — flat strings.
/// The registry is the authority on which ids are bound; deserialization
/// validates the id shape but is independent of registry membership.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LoopFamilyId(Cow<'static, str>);

impl LoopFamilyId {
    pub const DEFAULT: Self = Self(Cow::Borrowed("default"));
    // future Builtin families add consts here (e.g. `pub const CODING`)

    pub fn new(id: impl Into<Cow<'static, str>>) -> Result<Self, String> {
        let id = id.into();
        validate_loop_family_id(id.as_ref())?;
        Ok(Self(id))
    }

    pub fn as_str(&self) -> &str { self.0.as_ref() }
}

impl std::fmt::Display for LoopFamilyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(self.as_str()) }
}
```

### 3.2 `ComponentIdentity`

```rust
/// Content-addressed identity for a loop family, hook, skill snapshot,
/// or any other component whose version is load-bearing for replay.
///
/// One primitive across the system per zmanian's "four conventions" critique
/// Used today by `LoopFamily.version`; future hook /
/// skill-snapshot / model-route components should adopt the same shape.
///
/// `id` is human-readable and stable; `digest` content-hashes the underlying
/// composition. Bumping the composition (e.g. swapping a Default strategy in
/// a family) re-derives the digest and invalidates resume from older
/// checkpoints.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ComponentIdentity {
    pub id: Cow<'static, str>,
    pub digest: ComponentDigest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct ComponentDigest(pub [u8; 32]);   // blake3-32 over canonicalized composition

impl ComponentDigest {
    pub fn from_blake3(bytes: impl AsRef<[u8]>) -> Self {
        Self(*blake3::hash(bytes.as_ref()).as_bytes())
    }
}

impl ComponentIdentity {
    /// Constructs an identity for a family. The digest derivation policy is
    /// owned by the family factory; this constructor is just packaging.
    pub const fn from_static(id: &'static str, digest: ComponentDigest) -> Self {
        Self { id: Cow::Borrowed(id), digest }
    }

    pub fn new(id: impl Into<Cow<'static, str>>, digest: ComponentDigest) -> Self {
        Self { id: id.into(), digest }
    }
}
```

#### Migration / propagation

`ComponentIdentity` is the canonical primitive for component versioning **across the system**, not just for loop families. Per master doc §9, other version-carrying components migrate to this shape in their own owning PRs. This brief sets the target; the migrations themselves are out of scope here.

| Component | Today | Migration target | Migration cost | Owning PR |
|---|---|---|---|---|
| **Loop family** | `ComponentIdentity` (just landed in this brief) | already in target shape | — | this PR |
| **Checkpoint payload metadata** | references `ComponentIdentity` via the family's `version()` accessor | already in target shape | — | WS-0 + WS-10 |
| **Hooks** (`(HookId, hook_version)` tuple per [PR #3523 design](https://github.com/nearai/ironclaw/issues/3523)) | `(HookId, u32)` tuple — monotonic counter | `ComponentIdentity { id, digest }` — content-addressed | Content-hash the hook implementation at registration time. Hook PR adopts `ComponentIdentity` for its identity field. | #3524 PR |
| **Skill snapshot** (per [#3470](https://github.com/nearai/ironclaw/issues/3470)) | SHA-256 over length-prefixed snapshot bytes | `ComponentIdentity { id: skill_name, digest: existing_sha256 (truncated to 32 bytes — already 32 bytes for sha-256 raw) }` | Rename / re-wrap existing digest in the canonical struct. No recomputation needed; the SHA-256 already content-addresses the snapshot. | #3470 follow-up |
| **Model route** | `auth_version: String`, `config_version: String` on `LoopModelRouteSnapshot` ([`crates/ironclaw_turns/src/run_profile/host.rs:362`](../../../crates/ironclaw_turns/src/run_profile/host.rs)) — String identities, NOT content hashes | `auth: ComponentIdentity`, `config: ComponentIdentity` | Non-trivial: compute digests at version-mint time (content-hash the resolved route config / auth blob) and emit alongside the existing strings for one release before dropping the strings. | #3462 follow-up |

**This PR (#3544) does not migrate any of the existing fields.** The target shape is committed in the spec so the future #3470 / #3462 / #3524 PRs have a known migration destination; each migration ships in its own PR. The content-addressed-only requirement (master doc §9) means model-route migration is the most involved — current `String` identities need to be replaced by content digests, not just re-wrapped.

### 3.3 `LoopFamily`

```rust
use std::sync::Arc;
use crate::planner::AgentLoopPlanner;

/// A Builtin loop family — opaque to downstream crates. Holds the planner
/// (which holds the nine sealed strategies). Constructible only inside this
/// crate (via `pub(crate) fn new` invoked by `families::*` factories).
///
/// Downstream crates (`ironclaw_reborn`) can hold an `Arc<LoopFamily>` and
/// hand it to `PlannedDriver`, but cannot read strategies out of it — the
/// `pub(crate) fn planner` accessor is invisible outside this crate. The
/// canonical executor (also in this crate) is the only consumer of
/// `family.planner()`.
pub struct LoopFamily {
    id: LoopFamilyId,
    version: ComponentIdentity,
    planner: Arc<dyn AgentLoopPlanner>,
}

impl LoopFamily {
    /// Crate-private constructor. Family factories in `families::*` are the
    /// only callers. No public constructor exists — extensions cannot mint
    /// `LoopFamily` instances.
    pub(crate) fn new(
        id: LoopFamilyId,
        version: ComponentIdentity,
        planner: Arc<dyn AgentLoopPlanner>,
    ) -> Self {
        Self { id, version, planner }
    }

    pub fn id(&self) -> &LoopFamilyId { &self.id }
    pub fn version(&self) -> &ComponentIdentity { &self.version }

    /// Crate-private accessor used by the canonical executor. Not visible
    /// outside `ironclaw_agent_loop`.
    pub(crate) fn planner(&self) -> &dyn AgentLoopPlanner { self.planner.as_ref() }
}
```

### 3.4 `LoopFamilyRegistry`

```rust
use std::collections::HashMap;
use std::sync::Arc;

/// Guice-style singleton registry. Built once at startup by
/// `ironclaw_reborn`'s composition root, shared via `Arc<Self>`, immutable
/// thereafter.
///
/// There is intentionally NO public `register()` method and NO `Builder` —
/// the set of families is fixed at compile time. Adding a family means
/// editing the composition root (`ironclaw_reborn::app_loop_family`) to
/// call a new `families::*` factory.
pub struct LoopFamilyRegistry {
    families: HashMap<LoopFamilyId, Arc<LoopFamily>>,
}

impl LoopFamilyRegistry {
    /// Resolution. Production lookup; returns `None` for unbound ids so
    /// callers can produce a runner-side `Error::UnknownLoopFamily`.
    pub fn get(&self, id: &LoopFamilyId) -> Option<Arc<LoopFamily>> {
        self.families.get(id).cloned()
    }

    /// Returns the bound ids. Useful for diagnostics and registry-state
    /// observability tests.
    pub fn ids(&self) -> impl Iterator<Item = &LoopFamilyId> { self.families.keys() }

    /// Constructs a registry containing exactly the provided families and
    /// rejects duplicate ids. This is the only public registry constructor;
    /// production calls it from `ironclaw_reborn::app_loop_family`.
    pub fn with_families(
        families: Vec<Arc<LoopFamily>>,
    ) -> Result<Arc<Self>, LoopFamilyRegistryError> {
        let mut map = HashMap::with_capacity(families.len());
        for f in families {
            if map.contains_key(f.id()) {
                return Err(LoopFamilyRegistryError::DuplicateFamilyId {
                    id: f.id().clone(),
                });
            }
            map.insert(f.id().clone(), f);
        }
        Ok(Arc::new(Self { families: map }))
    }
}
```

### 3.5 `families::default`

```rust
//! crates/ironclaw_agent_loop/src/families/mod.rs

use std::sync::Arc;

use crate::default_planner::DefaultPlanner;
use crate::family::{ComponentDigest, ComponentIdentity, LoopFamily, LoopFamilyId};

/// The default loop family — text-tool-use baseline. Composes nine
/// `Default*Strategy` impls (WS-5) via `DefaultPlanner::compose` (the
/// `pub(crate)` constructor in WS-4). This is the one family the skeleton
/// ships.
///
/// Hypothetical future families (`routine`, `mission`, `coding`, `planning`)
/// would live alongside as additional `pub fn` exports in this module; each
/// composes the same `DefaultPlanner::compose` with a subset of overridden
/// strategies. See master doc §12.5 for the anticipated strategy-override
/// table.
pub fn default() -> LoopFamily {
    let planner: Arc<dyn crate::planner::AgentLoopPlanner> =
        Arc::new(DefaultPlanner::compose_default());
    LoopFamily::new(
        LoopFamilyId::DEFAULT,
        ComponentIdentity::from_static(
            "default",
            default_family_digest(),
        ),
        planner,
    )
}

#[cfg(test)]
const DEFAULT_FAMILY_FINGERPRINT: &[u8] =
    b"ironclaw_agent_loop.default_family.v1:planner=DefaultLoopFamilyPlanner;schema=component_identity_v1;family_id=default";

/// Stable digest: BLAKE3-256 of `DEFAULT_FAMILY_FINGERPRINT`.
///
/// Update this digest when the default family composition, planner behavior,
/// or identity schema changes in a replay-relevant way. Tests recompute the
/// BLAKE3 digest from `DEFAULT_FAMILY_FINGERPRINT` so this constant cannot
/// silently drift to another algorithm or stale preimage.
pub const DEFAULT_FAMILY_DIGEST: ComponentDigest = ComponentDigest([
    0x40, 0xe2, 0xeb, 0x31, 0x69, 0x81, 0x22, 0x31,
    0x39, 0x76, 0x00, 0x25, 0x49, 0x4a, 0x0e, 0x14,
    0xb5, 0xa1, 0x7a, 0x0a, 0x57, 0x59, 0x7d, 0xcd,
    0xa7, 0x48, 0xae, 0x38, 0x11, 0x75, 0xf8, 0x0f,
]);
```

### 3.6 Composition root in `ironclaw_reborn`

```rust
//! crates/ironclaw_reborn/src/app_loop_family.rs (NEW)

use std::sync::Arc;
use ironclaw_agent_loop::family::{LoopFamilyRegistry, LoopFamilyRegistryError};
use ironclaw_agent_loop::families;

/// Build the production loop-family registry. Called exactly once during
/// app startup; the resulting `Arc<LoopFamilyRegistry>` stays for the process
/// lifetime. Runner plumbing is deferred until the planned-driver and
/// run-profile selection workstreams land.
///
/// Adding a new family means adding a `families::<name>()` call in this
/// function — the only place that knows which families exist. The framework
/// crate (`ironclaw_agent_loop`) does NOT enumerate; it exports factories.
pub fn build_loop_family_registry() -> Result<Arc<LoopFamilyRegistry>, LoopFamilyRegistryError> {
    LoopFamilyRegistry::with_families(vec![
        Arc::new(families::default()),
        // future: Arc::new(families::coding()), Arc::new(families::routine()), ...
    ])
}
```

### 3.7 Deferred `TurnRunner` resolution path

`TurnRunner` resolution is intentionally not wired in this workstream. WS-3.5 establishes the registry type, the default family factory, and the Reborn composition root only. The runner-facing path lands with the planned-driver/run-profile workstreams after `PlannedDriver::from_family` and `ResolvedRunProfile.loop_family_id` exist.

The future target remains:

```rust
impl TurnRunner {
    pub fn new(
        host: Arc<dyn AgentLoopDriverHost>,
        loop_family_registry: Arc<LoopFamilyRegistry>,
        executor: Arc<CanonicalAgentLoopExecutor>,
        // ... existing fields ...
    ) -> Self { ... }

    async fn drive_run(&self, profile: &ResolvedRunProfile, ...) -> Result<LoopExit, Error> {
        let family = self
            .loop_family_registry
            .get(&profile.loop_family_id)
            .ok_or(Error::UnknownLoopFamily {
                id: profile.loop_family_id.clone(),
            })?;
        let driver = PlannedDriver::from_family(
            profile.loop_driver.id.clone(),
            family,
            self.executor.clone(),
            profile.loop_driver.version,
        );
        driver.run(self.host.clone(), /* request */).await
    }
}
```

`ResolvedRunProfile` gains `loop_family_id: LoopFamilyId` per the master doc §4.5 amendment in the owning run-profile migration. This is the field-rename/replacement for what would otherwise have been a `PlannerId`-style reference.

### 3.8 Sealed-trait enforcement check

The framework crate's strategy traits become `pub(crate)` in WS-1/2/3 (separate briefs handle the visibility flip). This brief adds a compile-time confirmation:

```rust
// crates/ironclaw_agent_loop/tests/sealing.rs (NEW)
//
// Compile-only test: confirm that strategy traits are not visible outside
// the crate. This file is in the `tests/` integration-test directory, which
// is treated as an external crate, so any `use` of a `pub(crate)` strategy
// trait will fail to compile.

#[test]
fn strategy_traits_are_sealed() {
    // Affirmative: LoopFamily, LoopFamilyId, ComponentIdentity ARE visible.
    fn _check(_id: ironclaw_agent_loop::family::LoopFamilyId) {}
    fn _check2(_family: ironclaw_agent_loop::family::LoopFamily) {}

    // The following lines, if uncommented, MUST fail to compile:
    //   use ironclaw_agent_loop::strategies::ContextStrategy;
    //   use ironclaw_agent_loop::strategies::StopConditionStrategy;
    //
    // (Compile-fail tests via `trybuild` are out of scope here; the comment
    // documents the invariant. WS-1/2/3 verification owns the actual
    // `pub(crate)` annotation.)
}
```

## 4. Acceptance criteria

- [ ] `cargo check -p ironclaw_agent_loop` passes
- [ ] `cargo check -p ironclaw_reborn` passes (composition root compiles)
- [ ] `cargo clippy --all --benches --tests --examples --all-features` zero warnings
- [ ] Unit tests in `ironclaw_agent_loop`:
  - [ ] `LoopFamilyId::DEFAULT.as_str() == "default"`
  - [ ] `LoopFamilyId` round-trips through `serde_json` as a flat string
  - [ ] `ComponentIdentity` round-trips through `serde_json`
  - [ ] `families::default().id() == &LoopFamilyId::DEFAULT`
  - [ ] `families::default().version().id == "default"`
- [ ] Unit tests in `ironclaw_reborn`:
  - [ ] `build_loop_family_registry()?.get(&LoopFamilyId::DEFAULT).is_some()`
  - [ ] `build_loop_family_registry()?.get(&LoopFamilyId::new("unknown")?).is_none()`
- [ ] Negative invariants (manual review checklist; no compile-fail harness in this brief):
  - [ ] No `pub fn new` on `LoopFamily` outside `ironclaw_agent_loop`
  - [ ] No `pub fn register` on `LoopFamilyRegistry`
  - [ ] No `Builder`-style mutation on `LoopFamilyRegistry`
  - [ ] No `pub` strategy trait re-export at `ironclaw_agent_loop::strategies::*`
- [ ] `with_families` is the only public registry constructor; production calling code lives in `ironclaw_reborn::app_loop_family`, not in `ironclaw_agent_loop`
- [ ] No `unwrap()` / `expect()` outside test code

## 5. Out of scope

- Loop families beyond `default` — out of skeleton scope; future families are factory functions in `families::*` added when consumers materialize
- `PlannedDriver::from_family` body — WS-7's amended brief owns the constructor; this brief only specifies the resolution call site
- Strategy-trait visibility flip from `pub` to `pub(crate)` — WS-1/2/3 own that
- `AgentLoopPlanner` trait-sealing pattern — WS-4 owns that; this brief assumes its presence
- `ResolvedRunProfile` schema change adding `loop_family_id` — out-of-band runner-side migration, tracked separately from the skeleton workstreams
- Compile-fail trybuild tests confirming the seal — useful but separate; the `pub(crate)` annotations + manual review checklist (above) are sufficient for the skeleton

## 6. Verification command sequence

```bash
cargo check -p ironclaw_agent_loop
cargo check -p ironclaw_reborn
cargo clippy --all --benches --tests --examples --all-features -- -D warnings
cargo test -p ironclaw_agent_loop
cargo test -p ironclaw_reborn
```

All five must succeed. The seal is enforced primarily through Rust visibility annotations landed in WS-1/2/3/4; this brief's tests confirm the registry plumbing without re-asserting the seal.
