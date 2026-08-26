use ironclaw_filesystem::{
    DiskDirectoryCapability, MAX_ORDINARY_HOST_TREE_DEPTH, inspect_ordinary_host_tree,
    read_ordinary_host_file,
};
use ironclaw_host_api::path::HostPath;

fn inspect(path: &std::path::Path) -> std::io::Result<bool> {
    inspect_ordinary_host_tree(&HostPath::from_path_buf(path.to_path_buf()))
}

#[test]
fn ordinary_host_tree_reports_whether_any_regular_file_exists() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("tree");
    std::fs::create_dir_all(root.join("empty/nested")).expect("empty tree");

    assert!(!inspect(&root).expect("empty ordinary tree validates"));

    std::fs::write(root.join("empty/nested/file.txt"), b"contents").expect("ordinary file");
    assert!(inspect(&root).expect("populated ordinary tree validates"));
}

#[test]
fn ordinary_host_tree_accepts_a_file_exactly_at_the_fixed_depth_bound() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("tree");
    std::fs::create_dir(&root).expect("tree root");
    let mut parent = root.clone();
    for level in 1..MAX_ORDINARY_HOST_TREE_DEPTH {
        parent = parent.join(format!("level-{level}"));
        std::fs::create_dir(&parent).expect("nested directory");
    }
    std::fs::write(parent.join("payload.txt"), b"payload").expect("depth-bound file");

    assert!(inspect(&root).expect("the inclusive maximum depth is accepted"));
}

#[test]
fn ordinary_host_tree_rejects_input_beyond_the_fixed_depth_bound() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("tree");
    std::fs::create_dir(&root).expect("tree root");
    let mut deepest = root.clone();
    for level in 0..=MAX_ORDINARY_HOST_TREE_DEPTH {
        deepest = deepest.join(format!("level-{level}"));
        std::fs::create_dir(&deepest).expect("nested directory");
    }

    let error = inspect(&root).expect_err("unbounded host tree must fail closed");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("depth"), "{error}");
}

#[cfg(unix)]
#[test]
fn ordinary_host_tree_rejects_root_and_nested_symlinks() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("tempdir");
    let outside = temp.path().join("outside");
    std::fs::create_dir(&outside).expect("outside directory");
    std::fs::write(outside.join("file.txt"), b"outside").expect("outside file");

    let root_alias = temp.path().join("root-alias");
    symlink(&outside, &root_alias).expect("root symlink");
    let root_error = inspect(&root_alias).expect_err("root symlink must fail closed");
    assert_eq!(root_error.kind(), std::io::ErrorKind::InvalidData);
    assert!(root_error.to_string().contains("symlink"), "{root_error}");

    let tree = temp.path().join("tree");
    std::fs::create_dir(&tree).expect("tree directory");
    symlink(outside.join("file.txt"), tree.join("nested-alias")).expect("nested symlink");
    let nested_error = inspect(&tree).expect_err("nested symlink must fail closed");
    assert_eq!(nested_error.kind(), std::io::ErrorKind::InvalidData);
    assert!(
        nested_error.to_string().contains("symlink"),
        "{nested_error}"
    );
}

#[cfg(unix)]
#[test]
fn ordinary_host_tree_rejects_non_file_non_directory_entries() {
    use std::os::unix::net::UnixListener;

    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("tree");
    std::fs::create_dir(&root).expect("tree directory");
    let _socket = UnixListener::bind(root.join("socket")).expect("unix socket");

    let error = inspect(&root).expect_err("socket must fail closed");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("ordinary"), "{error}");
}

#[cfg(unix)]
#[test]
fn ordinary_host_file_read_rejects_a_symlink_instead_of_following_it() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("tempdir");
    let outside = temp.path().join("outside.txt");
    std::fs::write(&outside, b"outside").expect("outside file");
    let selected = temp.path().join("selected.txt");
    symlink(&outside, &selected).expect("selected symlink");

    let root = DiskDirectoryCapability::admit_existing(temp.path()).expect("retain test root");
    let error = read_ordinary_host_file(&root, std::path::Path::new("selected.txt"), 1024)
        .expect_err("verified read must not follow a replacement symlink");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("symlink"), "{error}");
}

#[test]
fn ordinary_host_file_read_rejects_bytes_beyond_the_caller_limit() {
    let temp = tempfile::tempdir().expect("tempdir");
    let selected = temp.path().join("selected.txt");
    std::fs::write(&selected, b"12345").expect("oversized selected file");

    let root = DiskDirectoryCapability::admit_existing(temp.path()).expect("retain test root");
    let error = read_ordinary_host_file(&root, std::path::Path::new("selected.txt"), 4)
        .expect_err("verified read must stay within the caller's byte limit");

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("byte limit"), "{error}");
}
