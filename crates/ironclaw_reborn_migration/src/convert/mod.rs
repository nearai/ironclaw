//! Per-domain converters. Each `run(src, tgt, options, report)` reads one v1
//! domain and writes the Reborn equivalent, recording losses on `report`.

pub(crate) mod automations;
pub(crate) mod extensions;
pub(crate) mod heartbeat;
pub(crate) mod identities;
pub(crate) mod jobs;
pub(crate) mod memory;
pub(crate) mod secrets;
pub(crate) mod settings;
pub(crate) mod threads;
