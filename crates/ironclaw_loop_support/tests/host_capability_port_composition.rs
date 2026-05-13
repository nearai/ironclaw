use std::path::{Path, PathBuf};

#[test]
fn no_external_construction_of_host_runtime_capability_port() {
    let workspace_root = workspace_root();
    let mut offenders = Vec::new();
    visit_rs_files(&workspace_root, &mut |path| {
        if should_skip(path) {
            return;
        }
        let src = std::fs::read_to_string(path).unwrap_or_default();
        if src.contains("HostRuntimeLoopCapabilityPort::new(")
            || src.contains("HostRuntimeLoopCapabilityPort {")
        {
            offenders.push(path.display().to_string());
        }
    });

    assert!(
        offenders.is_empty(),
        "HostRuntimeLoopCapabilityPort must be constructed only inside ironclaw_loop_support; offenders: {offenders:#?}"
    );
}

fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .expect("crate lives under workspace crates directory")
}

fn visit_rs_files(root: &Path, visit: &mut impl FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            visit_rs_files(&path, visit);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            visit(&path);
        }
    }
}

fn should_skip(path: &Path) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str();
        name == "ironclaw_loop_support" || name == "tests" || name == "target"
    })
}
