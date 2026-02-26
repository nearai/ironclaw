//! Progress reporting for import operations.

/// Trait for reporting import progress to the user.
pub trait ImportProgress: Send {
    /// Start a named import phase (e.g., "Settings", "Memory documents").
    fn start_phase(&mut self, name: &str, total: usize);
    /// Report that an item was successfully imported.
    fn item_imported(&mut self, name: &str);
    /// Report that an item was skipped.
    fn item_skipped(&mut self, name: &str, reason: &str);
    /// Report that an item failed.
    fn item_error(&mut self, name: &str, error: &str);
    /// End the current phase.
    fn end_phase(&mut self);
}

/// Progress reporter that prints to stdout.
pub struct TerminalProgress {
    phase_name: String,
    phase_total: usize,
    phase_done: usize,
}

impl Default for TerminalProgress {
    fn default() -> Self {
        Self::new()
    }
}

impl TerminalProgress {
    pub fn new() -> Self {
        Self {
            phase_name: String::new(),
            phase_total: 0,
            phase_done: 0,
        }
    }
}

impl ImportProgress for TerminalProgress {
    fn start_phase(&mut self, name: &str, total: usize) {
        self.phase_name = name.to_string();
        self.phase_total = total;
        self.phase_done = 0;
        if total > 0 {
            println!("  Importing {} ({} items)...", name, total);
        } else {
            println!("  Importing {}...", name);
        }
    }

    fn item_imported(&mut self, name: &str) {
        self.phase_done += 1;
        println!("    + {}", name);
    }

    fn item_skipped(&mut self, name: &str, reason: &str) {
        self.phase_done += 1;
        println!("    - {} (skipped: {})", name, reason);
    }

    fn item_error(&mut self, name: &str, error: &str) {
        self.phase_done += 1;
        eprintln!("    ! {} (error: {})", name, error);
    }

    fn end_phase(&mut self) {
        if self.phase_total > 0 {
            println!(
                "  Done: {}/{} {}",
                self.phase_done, self.phase_total, self.phase_name
            );
        }
    }
}

/// Silent progress reporter (no-op). Used for tests.
pub struct SilentProgress;

impl ImportProgress for SilentProgress {
    fn start_phase(&mut self, _name: &str, _total: usize) {}
    fn item_imported(&mut self, _name: &str) {}
    fn item_skipped(&mut self, _name: &str, _reason: &str) {}
    fn item_error(&mut self, _name: &str, _error: &str) {}
    fn end_phase(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_progress_does_not_panic() {
        let mut p = TerminalProgress::new();
        p.start_phase("test", 3);
        p.item_imported("a");
        p.item_skipped("b", "exists");
        p.item_error("c", "broken");
        p.end_phase();
    }

    #[test]
    fn silent_progress_does_not_panic() {
        let mut p = SilentProgress;
        p.start_phase("test", 0);
        p.item_imported("x");
        p.item_skipped("y", "skip");
        p.item_error("z", "err");
        p.end_phase();
    }
}
