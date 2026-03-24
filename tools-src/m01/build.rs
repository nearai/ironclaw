use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=m01_tool.capabilities.json");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let caps_src = manifest_dir.join("m01_tool.capabilities.json");
    let stem = env::var("CARGO_PKG_NAME")
        .expect("package name")
        .replace('-', "_");

    let Some(profile_dir) = out_dir.ancestors().nth(3) else {
        panic!("unexpected OUT_DIR layout: {}", out_dir.display());
    };
    let caps_dst = profile_dir.join(format!("{stem}.capabilities.json"));

    fs::copy(&caps_src, &caps_dst).unwrap_or_else(|error| {
        panic!(
            "failed to copy capabilities file from {} to {}: {}",
            caps_src.display(),
            caps_dst.display(),
            error
        )
    });
}
