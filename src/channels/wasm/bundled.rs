//! Known WASM channels that can be installed from build artifacts.
//!
//! Instead of embedding WASM binaries in the host binary via include_bytes!,
//! channels are compiled separately and installed from their build output
//! directories during onboarding.
//!
//! Channel source layout:
//!   channels-src/<name>/
//!     target/wasm32-wasip2/release/<name>_channel.wasm
//!     <name>.capabilities.json

use std::path::{Path, PathBuf};

use tokio::fs;

/// Compile-time project root, used to locate channels-src/ in dev builds.
const CARGO_MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

/// Known channel names and their crate names (for locating build artifacts).
const KNOWN_CHANNELS: &[(&str, &str)] = &[
    ("telegram", "telegram_channel"),
    ("slack", "slack_channel"),
    ("discord", "discord_channel"),
    ("whatsapp", "whatsapp_channel"),
];

/// Names of known channels that can be installed.
pub fn bundled_channel_names() -> Vec<&'static str> {
    KNOWN_CHANNELS.iter().map(|(name, _)| *name).collect()
}

/// Resolve the channels source directory.
///
/// Checks (in order):
/// 1. `IRONCLAW_CHANNELS_SRC` env var
/// 2. `<CARGO_MANIFEST_DIR>/channels-src/` (dev builds)
fn channels_src_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("IRONCLAW_CHANNELS_SRC") {
        return PathBuf::from(dir);
    }
    PathBuf::from(CARGO_MANIFEST_DIR).join("channels-src")
}

/// Locate the build artifacts for a channel.
///
/// Checks two layouts:
/// 1. **Flat** (Docker/packaged): `<channels_src>/<name>/<name>.wasm`
/// 2. **Build tree** (dev): `<channels_src>/<name>/target/wasm32-wasip2/release/<crate_name>.wasm`
///
/// Returns (wasm_path, capabilities_path) or an error if files are missing.
fn locate_channel_artifacts(name: &str) -> Result<(PathBuf, PathBuf), String> {
    let (_, crate_name) = KNOWN_CHANNELS
        .iter()
        .find(|(n, _)| *n == name)
        .ok_or_else(|| format!("Unknown channel '{}'", name))?;

    let src_dir = channels_src_dir();
    let channel_dir = src_dir.join(name);

    let caps_path = channel_dir.join(format!("{}.capabilities.json", name));

    // Check flat layout first (Docker/packaged deployments)
    let flat_wasm = channel_dir.join(format!("{}.wasm", name));
    if flat_wasm.exists() && caps_path.exists() {
        return Ok((flat_wasm, caps_path));
    }

    // Fall back to build tree layout (dev builds) — search across all WASM triples
    if let Some(build_wasm) =
        crate::registry::artifacts::find_wasm_artifact(&channel_dir, crate_name, "release")
        && caps_path.exists()
    {
        return Ok((build_wasm, caps_path));
    }

    // Provide a helpful error with the paths we checked
    let expected_build = crate::registry::artifacts::resolve_target_dir(&channel_dir)
        .join("wasm32-wasip2/release")
        .join(format!("{}.wasm", crate_name));

    Err(format!(
        "Channel '{}' WASM not found. Checked:\n  \
         - {} (flat/packaged)\n  \
         - {} (build tree, and other triples)\n  \
         Build it first:\n  \
         cd {} && cargo build --target wasm32-wasip2 --release",
        name,
        flat_wasm.display(),
        expected_build.display(),
        channel_dir.display()
    ))
}

/// Install a channel from build artifacts into the channels directory.
pub async fn install_bundled_channel(
    name: &str,
    target_dir: &Path,
    force: bool,
) -> Result<(), String> {
    let (wasm_src, caps_src) = locate_channel_artifacts(name)?;

    fs::create_dir_all(target_dir)
        .await
        .map_err(|e| format!("Failed to create channels directory: {}", e))?;

    let wasm_dst = target_dir.join(format!("{}.wasm", name));
    let caps_dst = target_dir.join(format!("{}.capabilities.json", name));

    let has_existing = wasm_dst.exists() || caps_dst.exists();
    if has_existing && !force {
        return Err(format!(
            "Channel '{}' already exists at {}",
            name,
            target_dir.display()
        ));
    }

    fs::copy(&wasm_src, &wasm_dst)
        .await
        .map_err(|e| format!("Failed to copy {}: {}", wasm_src.display(), e))?;
    fs::copy(&caps_src, &caps_dst)
        .await
        .map_err(|e| format!("Failed to copy {}: {}", caps_src.display(), e))?;

    Ok(())
}

/// Check which known channels have build artifacts available.
pub fn available_channel_names() -> Vec<&'static str> {
    KNOWN_CHANNELS
        .iter()
        .filter(|(name, _)| locate_channel_artifacts(name).is_ok())
        .map(|(name, _)| *name)
        .collect()
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;
    use tokio::fs;

    use super::*;

    #[test]
    fn test_known_channels_includes_all_four() {
        let names = bundled_channel_names();
        assert!(names.contains(&"telegram"));
        assert!(names.contains(&"slack"));
        assert!(names.contains(&"discord"));
        assert!(names.contains(&"whatsapp"));
    }

    #[test]
    fn test_channels_src_dir_default() {
        let dir = channels_src_dir();
        assert!(dir.ends_with("channels-src"));
    }

    #[test]
    fn test_locate_unknown_channel_errors() {
        assert!(locate_channel_artifacts("nonexistent").is_err());
    }

    #[tokio::test]
    async fn test_install_refuses_overwrite_without_force() {
        let dir = tempdir().unwrap();
        let wasm_path = dir.path().join("telegram.wasm");
        fs::write(&wasm_path, b"custom").await.unwrap();

        let result = install_bundled_channel("telegram", dir.path(), false).await;
        // Either fails because artifacts missing OR because file exists
        assert!(result.is_err());

        // Original file should be untouched
        let existing = fs::read(&wasm_path).await.unwrap();
        assert_eq!(existing, b"custom");
    }
}
