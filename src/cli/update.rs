//! Core IronClaw update checker.
//!
//! This command is intentionally read-only. It checks GitHub Releases for the
//! latest stable IronClaw version and reports whether the running binary is
//! outdated. It does not mutate the installation.

use crate::release::{check_for_update, current_version};

pub async fn run_update_command() -> anyhow::Result<()> {
    println!("IronClaw Update Check");
    println!("=====================\n");

    let current = current_version()?;
    match check_for_update(current.clone()).await {
        Ok(check) => {
            println!("  Current:     v{}", check.current);
            println!("  Latest:      v{}", check.latest);

            if check.update_available() {
                println!("  Status:      update available");
                if let Some(name) = check.release_name {
                    println!("  Release:     {}", name);
                }
                println!("  Release URL: {}", check.release_url);
                println!(
                    "\n  IronClaw does not currently self-update in place. \
                     Use the platform-specific release installer to upgrade."
                );
            } else {
                println!("  Status:      up to date");
            }
        }
        Err(err) => {
            println!("  Current:     v{}", current);
            println!("  Status:      unable to check for updates");
            println!("  Error:       {}", err);
        }
    }

    Ok(())
}
