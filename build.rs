//! Build script: compile Telegram channel WASM from source.
//!
//! Do not commit compiled WASM binaries — they are a supply chain risk.
//! This script builds telegram.wasm from channels-src/telegram before the main crate compiles.
//!
//! Reproducible build:
//!   cargo build --release
//! (build.rs invokes the channel build automatically)
//!
//! Prerequisites: rustup target add wasm32-wasip2, cargo install wasm-tools

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let root = PathBuf::from(&manifest_dir);
    let channel_dir = root.join("channels-src/telegram");
    let wasm_out = channel_dir.join("telegram.wasm");

    // Rerun when channel source or build script changes
    println!("cargo:rerun-if-changed=channels-src/telegram/src");
    println!("cargo:rerun-if-changed=channels-src/telegram/Cargo.toml");
    println!("cargo:rerun-if-changed=wit/channel.wit");

    if !channel_dir.is_dir() {
        return;
    }

    // Build WASM module
    let status = match Command::new("cargo")
        .args([
            "build",
            "--release",
            "--target",
            "wasm32-wasip2",
            "--manifest-path",
            channel_dir.join("Cargo.toml").to_str().unwrap(),
        ])
        .current_dir(&root)
        .status()
    {
        Ok(s) => s,
        Err(_) => {
            eprintln!(
                "cargo:warning=Telegram channel build failed. Run: ./channels-src/telegram/build.sh"
            );
            return;
        }
    };

    if !status.success() {
        eprintln!(
            "cargo:warning=Telegram channel build failed. Run: ./channels-src/telegram/build.sh"
        );
        return;
    }

    let raw_wasm = channel_dir.join("target/wasm32-wasip2/release/telegram_channel.wasm");
    if !raw_wasm.exists() {
        eprintln!(
            "cargo:warning=Telegram WASM output not found at {:?}",
            raw_wasm
        );
        return;
    }

    // Convert to component and strip (wasm-tools)
    let component_ok = Command::new("wasm-tools")
        .args([
            "component",
            "new",
            raw_wasm.to_str().unwrap(),
            "-o",
            wasm_out.to_str().unwrap(),
        ])
        .current_dir(&root)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !component_ok {
        // Fallback: copy raw module if wasm-tools unavailable
        if std::fs::copy(&raw_wasm, &wasm_out).is_err() {
            eprintln!("cargo:warning=wasm-tools not found. Run: cargo install wasm-tools");
        }
    } else {
        // Strip debug info (use temp file to avoid clobbering)
        let stripped = wasm_out.with_extension("wasm.stripped");
        let strip_ok = Command::new("wasm-tools")
            .args([
                "strip",
                wasm_out.to_str().unwrap(),
                "-o",
                stripped.to_str().unwrap(),
            ])
            .current_dir(&root)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if strip_ok {
            let _ = std::fs::rename(&stripped, &wasm_out);
        }
    }
}
