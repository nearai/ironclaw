//! WASM runtime contracts for IronClaw Reborn.
//!
//! `ironclaw_wasm` validates and invokes portable WASM capabilities. Modules
//! receive no ambient host authority: every privileged effect must eventually
//! cross an explicit host import checked by IronClaw host API contracts.

use ironclaw_extensions::{ExtensionError, ExtensionPackage, ExtensionRuntime};
use ironclaw_filesystem::{FilesystemError, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityId, ExtensionId, MountView, NetworkMethod, NetworkPolicy,
    NetworkScheme, NetworkTarget, NetworkTargetPattern, ResourceEstimate, ResourceReservation,
    ResourceReservationId, ResourceScope, ResourceUsage, RuntimeKind, ScopedPath, VirtualPath,
};
use ironclaw_resources::{ResourceError, ResourceGovernor, ResourceReceipt};
use rust_decimal::Decimal;
use serde_json::Value;
use std::{
    collections::HashMap,
    future::Future,
    net::{IpAddr, ToSocketAddrs},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    sync::{Arc, Condvar, Mutex, OnceLock, Weak, mpsc},
    time::{Duration, Instant},
};
use thiserror::Error;
use wasmtime::{Cache, Caller, Config, Engine, Instance, Linker, Module, ResourceLimiter, Store};

include!("constants.rs");
include!("config.rs");
include!("filesystem.rs");
include!("network.rs");
include!("types.rs");
include!("runtime.rs");
include!("imports.rs");
include!("support.rs");

#[cfg(test)]
include!("review_hardening_tests.rs");
