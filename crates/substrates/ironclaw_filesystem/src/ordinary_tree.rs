use std::io::{self, Read as _};

use cap_fs_ext::{DirExt as _, FollowSymlinks, OpenOptionsFollowExt as _};
use cap_std::fs::{Dir, DirEntry};
use ironclaw_host_api::path::HostPath;
use same_file::Handle;

use crate::local_capability::DiskDirectoryCapability;

/// Maximum relative depth accepted by [`inspect_ordinary_host_tree`].
pub const MAX_ORDINARY_HOST_TREE_DEPTH: usize = 64;

/// Validates that a host tree contains only regular files and directories.
///
/// The walk rejects symlinks and special entries, stays relative to retained
/// directory handles, and verifies that every directory name still resolves to
/// the handle that was traversed. The boolean reports whether at least one
/// regular file was found.
pub fn inspect_ordinary_host_tree(root: &HostPath) -> io::Result<bool> {
    let path = root.as_path();
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(invalid_tree("ordinary host tree contains a symlink"));
    }
    if metadata.is_file() {
        return Ok(true);
    }
    if !metadata.is_dir() {
        return Err(invalid_tree(
            "ordinary host tree contains a non-file, non-directory entry",
        ));
    }

    let directory = open_directory_no_follow(path)
        .map_err(|error| with_context("open ordinary host tree root", error))?;
    let contains_files = inspect_directory(&directory, 0)?;
    let reopened = open_directory_no_follow(path)
        .map_err(|error| with_context("reopen ordinary host tree root", error))?;
    ensure_same_directory(&directory, &reopened)?;
    Ok(contains_files)
}

fn open_directory_no_follow(path: &std::path::Path) -> io::Result<Dir> {
    let Some(parent) = path.parent() else {
        return DiskDirectoryCapability::open_existing_no_follow(path)
            .and_then(|capability| capability.directory().try_clone());
    };
    let Some(file_name) = path.file_name() else {
        return DiskDirectoryCapability::open_existing_no_follow(path)
            .and_then(|capability| capability.directory().try_clone());
    };
    // Resolve ambient ancestor aliases (for example macOS `/var` ->
    // `/private/var`) while keeping the selected root as the no-follow final
    // component of a retained parent capability.
    let canonical_parent = parent.canonicalize()?;
    let parent = DiskDirectoryCapability::open_existing_no_follow(&canonical_parent)?;
    parent.directory().open_dir_nofollow(file_name)
}

/// Reads a regular host file through a no-follow descriptor open.
///
/// Every path component is resolved relative to the caller's retained root
/// capability. Ancestor directories and the final component are opened
/// without following symlinks, and file type and caller-supplied byte limit
/// are verified from the open handle before any bytes are read. The limit is
/// enforced again while reading so concurrent growth stays bounded.
pub fn read_ordinary_host_file(
    root: &DiskDirectoryCapability,
    relative_path: &std::path::Path,
    max_bytes: usize,
) -> io::Result<Vec<u8>> {
    let parent_path = relative_path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "ordinary host file must have a capability-relative parent",
        )
    })?;
    let file_name = relative_path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "ordinary host file must have a final path component",
        )
    })?;
    let parent = root.open_existing_directory(parent_path).map_err(|error| {
        with_context(
            "open ordinary host file parent without following links",
            error,
        )
    })?;
    // Give a symlink selected at inspection time a stable InvalidData error.
    // The no-follow open below remains necessary to reject a replacement
    // symlink installed after this descriptor-relative metadata check.
    let selected_metadata = parent
        .symlink_metadata(file_name)
        .map_err(|error| with_context("inspect ordinary host file entry", error))?;
    if selected_metadata.file_type().is_symlink() {
        return Err(invalid_tree("ordinary host file is a symlink"));
    }
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = parent
        .open_with(file_name, &options)
        .map_err(|error| with_context("open ordinary host file without following links", error))?;
    let metadata = file
        .metadata()
        .map_err(|error| with_context("inspect opened ordinary host file", error))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(invalid_tree(
            "ordinary host file is a symlink or non-regular entry",
        ));
    }
    let max_bytes_u64 = u64::try_from(max_bytes).unwrap_or(u64::MAX);
    if metadata.len() > max_bytes_u64 {
        return Err(invalid_tree("ordinary host file exceeds byte limit"));
    }
    read_to_end_bounded(
        file,
        max_bytes,
        usize::try_from(metadata.len()).unwrap_or(max_bytes),
    )
}

fn read_to_end_bounded(
    reader: impl std::io::Read,
    max_bytes: usize,
    initial_capacity: usize,
) -> io::Result<Vec<u8>> {
    let max_bytes_u64 = u64::try_from(max_bytes).unwrap_or(u64::MAX);
    let mut bytes = Vec::with_capacity(initial_capacity.min(max_bytes));
    reader
        .take(max_bytes_u64.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| with_context("read opened ordinary host file", error))?;
    if bytes.len() > max_bytes {
        return Err(invalid_tree("ordinary host file exceeds byte limit"));
    }
    Ok(bytes)
}

fn inspect_directory(directory: &Dir, depth: usize) -> io::Result<bool> {
    inspect_directory_with_hook(directory, depth, &mut |_| {})
}

fn inspect_directory_with_hook(
    directory: &Dir,
    depth: usize,
    before_directory_open: &mut impl FnMut(&DirEntry),
) -> io::Result<bool> {
    ensure_depth(depth)?;
    let mut contains_files = false;
    for entry in directory
        .read_dir(".")
        .map_err(|error| with_context("read ordinary host tree directory", error))?
    {
        let entry = entry.map_err(|error| with_context("read ordinary host tree entry", error))?;
        let entry_depth = depth.saturating_add(1);
        ensure_depth(entry_depth)?;
        let file_type = entry
            .file_type()
            .map_err(|error| with_context("inspect ordinary host tree entry", error))?;
        if file_type.is_symlink() {
            return Err(invalid_tree("ordinary host tree contains a symlink"));
        }
        if file_type.is_file() {
            contains_files = true;
            continue;
        }
        if !file_type.is_dir() {
            return Err(invalid_tree(
                "ordinary host tree contains a non-file, non-directory entry",
            ));
        }

        before_directory_open(&entry);
        let entry_name = entry.file_name();
        let child = directory.open_dir_nofollow(&entry_name).map_err(|error| {
            with_context(
                "open ordinary host tree directory without following symlinks",
                error,
            )
        })?;
        contains_files |= inspect_directory_with_hook(&child, entry_depth, before_directory_open)?;
        ensure_entry_still_names_directory(directory, &entry_name, &child)?;
    }
    Ok(contains_files)
}

fn ensure_entry_still_names_directory(
    parent: &Dir,
    entry_name: &std::ffi::OsStr,
    opened: &Dir,
) -> io::Result<()> {
    let reopened = parent.open_dir_nofollow(entry_name).map_err(|error| {
        with_context(
            "reopen ordinary host tree directory without following symlinks",
            error,
        )
    })?;
    ensure_same_directory(opened, &reopened)
}

fn ensure_same_directory(first: &Dir, second: &Dir) -> io::Result<()> {
    let first_handle = Handle::from_file(first.try_clone()?.into_std_file())?;
    let second_handle = Handle::from_file(second.try_clone()?.into_std_file())?;
    if first_handle == second_handle {
        Ok(())
    } else {
        Err(invalid_tree(
            "ordinary host tree directory identity changed during traversal",
        ))
    }
}

fn ensure_depth(depth: usize) -> io::Result<()> {
    if depth <= MAX_ORDINARY_HOST_TREE_DEPTH {
        Ok(())
    } else {
        Err(invalid_tree(
            "ordinary host tree exceeds maximum traversal depth",
        ))
    }
}

fn invalid_tree(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn with_context(context: &'static str, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{context}: {error}"))
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::sync::{Arc, Barrier};

    use cap_std::{ambient_authority, fs::Dir};

    use super::{ensure_same_directory, inspect_directory_with_hook, read_to_end_bounded};

    #[test]
    fn bounded_read_rejects_bytes_added_after_the_metadata_check() {
        let reader = std::io::Cursor::new(b"12345");

        let error = read_to_end_bounded(reader, 4, 4)
            .expect_err("post-metadata growth must stay within the caller's byte limit");

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("byte limit"), "{error}");
    }

    #[test]
    fn directory_identity_check_rejects_different_handles() {
        let first = tempfile::tempdir().expect("first tempdir");
        let second = tempfile::tempdir().expect("second tempdir");
        let first = Dir::open_ambient_dir(first.path(), ambient_authority()).expect("first dir");
        let second = Dir::open_ambient_dir(second.path(), ambient_authority()).expect("second dir");

        let error = ensure_same_directory(&first, &second)
            .expect_err("different directories must fail the identity check");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("identity"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn directory_swap_to_symlink_between_enumeration_and_open_fails_closed() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("tempdir");
        let root_path = temp.path().join("root");
        let child_path = root_path.join("child");
        let retired_path = root_path.join("retired");
        let alternate_path = root_path.join("alternate");
        std::fs::create_dir_all(&child_path).expect("child directory");
        std::fs::create_dir(&alternate_path).expect("alternate directory");
        std::fs::write(alternate_path.join("alternate.txt"), b"alternate").expect("alternate file");
        let root = Dir::open_ambient_dir(&root_path, ambient_authority()).expect("root dir");

        let enumerated = Arc::new(Barrier::new(2));
        let replaced = Arc::new(Barrier::new(2));
        let replacer = {
            let enumerated = Arc::clone(&enumerated);
            let replaced = Arc::clone(&replaced);
            std::thread::spawn(move || {
                enumerated.wait();
                std::fs::rename(child_path, retired_path).expect("retire child directory");
                symlink(alternate_path, root_path.join("child"))
                    .expect("replace child with symlink");
                replaced.wait();
            })
        };

        let mut swapped = false;
        let error = inspect_directory_with_hook(&root, 0, &mut |entry| {
            if !swapped && entry.file_name() == "child" {
                swapped = true;
                enumerated.wait();
                replaced.wait();
            }
        })
        .expect_err("a directory replaced by a symlink must fail closed");
        replacer.join().expect("replacer thread");

        assert!(swapped, "test must intercept the selected child directory");
        assert!(
            error.to_string().contains("without following symlinks"),
            "{error}"
        );
    }
}
