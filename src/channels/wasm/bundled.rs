//! Bundled WASM channels that can be installed locally.

use std::path::Path;

use tokio::fs;

#[derive(Clone, Copy)]
struct BundledChannel {
    name: &'static str,
    wasm: &'static [u8],
    capabilities: &'static [u8],
}

/// Names of bundled channels shipped with IronClaw.
pub fn bundled_channel_names() -> &'static [&'static str] {
    &["telegram"]
}

/// Install a bundled channel into a channels directory.
pub async fn install_bundled_channel(
    name: &str,
    target_dir: &Path,
    force: bool,
) -> Result<(), String> {
    let channel = bundled_channel(name)
        .ok_or_else(|| format!("Unknown bundled channel '{}'", name.to_lowercase()))?;

    fs::create_dir_all(target_dir)
        .await
        .map_err(|e| format!("Failed to create channels directory: {}", e))?;

    let wasm_path = target_dir.join(format!("{}.wasm", channel.name));
    let caps_path = target_dir.join(format!("{}.capabilities.json", channel.name));

    let has_existing = wasm_path.exists() || caps_path.exists();
    if has_existing && !force {
        return Err(format!(
            "Channel '{}' already exists at {}",
            channel.name,
            target_dir.display()
        ));
    }

    fs::write(&wasm_path, channel.wasm)
        .await
        .map_err(|e| format!("Failed to write {}: {}", wasm_path.display(), e))?;
    fs::write(&caps_path, channel.capabilities)
        .await
        .map_err(|e| format!("Failed to write {}: {}", caps_path.display(), e))?;

    Ok(())
}

fn bundled_channel(name: &str) -> Option<BundledChannel> {
    if name.eq_ignore_ascii_case("telegram") {
        Some(BundledChannel {
            name: "telegram",
            wasm: include_bytes!("../../../channels-src/telegram/telegram.wasm"),
            capabilities: include_bytes!(
                "../../../channels-src/telegram/telegram.capabilities.json"
            ),
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;
    use tokio::fs;

    use super::*;

    #[test]
    fn test_bundled_channel_names_contains_telegram() {
        assert!(bundled_channel_names().contains(&"telegram"));
    }

    #[tokio::test]
    async fn test_install_bundled_channel_writes_files() {
        let dir = tempdir().unwrap();

        install_bundled_channel("telegram", dir.path(), false)
            .await
            .unwrap();

        assert!(dir.path().join("telegram.wasm").exists());
        assert!(dir.path().join("telegram.capabilities.json").exists());
    }

    #[tokio::test]
    async fn test_install_bundled_channel_refuses_overwrite_without_force() {
        let dir = tempdir().unwrap();
        let wasm_path = dir.path().join("telegram.wasm");
        fs::write(&wasm_path, b"custom").await.unwrap();

        let result = install_bundled_channel("telegram", dir.path(), false).await;
        assert!(result.is_err());

        let existing = fs::read(&wasm_path).await.unwrap();
        assert_eq!(existing, b"custom");
    }
}
