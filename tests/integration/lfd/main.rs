//! LFD eval runner — the data-driven batch executor for the loss-function
//! development portfolio (`lfd/_shared/SCHEMA.md`).
//!
//! One `#[tokio::test]` (`lfd_runner`) that is a NO-OP unless `LFD_CASES` is
//! set. When set, every `*.json` Case file under `$LFD_CASES` is executed
//! through the Reborn integration harness (the profile named by the case
//! assembles it) and one `<case_id>.outcome.json` per the SCHEMA.md §2 Outcome
//! shape is written to `$LFD_OUT`. A bad case never kills the batch: parse,
//! assembly, and turn failures become `status: "error"` outcomes; cases a
//! profile cannot execute become `status: "unsupported"`.
//!
//! Trust boundary (SCHEMA.md §6): everything in this directory EXCEPT
//! `profiles/<feature>.rs` is pinned runner code — outcome extraction, the
//! state-query dispatcher, the leak scan, and the runner hash all live here,
//! not in profiles, so a profile cannot fabricate outcomes through the
//! supported API.

#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../../support/mod.rs"]
mod support;

mod case;
mod extract;
mod leak_scan;
mod outcome;
mod profiles;
mod runner;
mod runner_hash;

use std::path::Path;

/// Data-driven LFD batch runner. No-op (vacuously green) without `LFD_CASES`
/// so the plain CI test sweep never depends on eval-case availability; the LFD
/// scorer invokes it with both env vars set.
#[tokio::test]
async fn lfd_runner() {
    let Ok(cases_dir) = std::env::var("LFD_CASES") else {
        return;
    };
    let out_dir = std::env::var("LFD_OUT").expect("LFD_OUT must be set when LFD_CASES is set");
    runner::run_batch(Path::new(&cases_dir), Path::new(&out_dir))
        .await
        .expect("lfd batch runner failed");
}
