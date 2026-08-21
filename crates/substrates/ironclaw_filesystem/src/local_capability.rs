use std::{
    ffi::OsString,
    io::{self, Read as _, Write as _},
    path::{Component, Path},
    sync::Arc,
    time::SystemTime,
};

use cap_fs_ext::{
    DirExt as _, FollowSymlinks, OpenOptionsFollowExt as _, OpenOptionsMaybeDirExt as _,
};
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use same_file::Handle;

use crate::CasExpectation;

pub(crate) async fn run_capability_blocking<T, F>(work: F) -> io::Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> io::Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(io::Error::other)?
}

pub(crate) async fn run_capability_access<T, F>(work: F) -> Result<T, CapabilityWriteError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, CapabilityWriteError> + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| CapabilityWriteError::Io(io::Error::other(error)))?
}

#[derive(Debug, Clone)]
pub struct DiskDirectoryCapability {
    directory: Arc<Dir>,
}

#[derive(Debug)]
pub(crate) struct CapabilityDirectoryEntry {
    pub(crate) name: OsString,
    pub(crate) file_type: CapabilityFileType,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum CapabilityFileType {
    File,
    Directory,
    Other,
}

#[derive(Debug)]
pub(crate) struct CapabilityMetadata {
    pub(crate) file_type: CapabilityFileType,
    pub(crate) len: u64,
    pub(crate) modified: Option<SystemTime>,
}

impl DiskDirectoryCapability {
    pub fn admit_or_create(path: &Path) -> io::Result<Self> {
        if !path.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "local capability root must be absolute",
            ));
        }
        let (anchor, tail) = absolute_anchor_and_tail(path)?;
        let mut directory = Dir::open_ambient_dir(anchor, ambient_authority())?;
        for component in tail {
            directory = match directory.open_dir_nofollow(&component) {
                Ok(child) => child,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    match directory.create_dir(&component) {
                        Ok(()) => {}
                        Err(create_error)
                            if create_error.kind() == io::ErrorKind::AlreadyExists => {}
                        Err(create_error) => return Err(create_error),
                    }
                    directory.open_dir_nofollow(&component)?
                }
                Err(error) => return Err(error),
            };
        }
        Ok(Self {
            directory: Arc::new(directory),
        })
    }

    pub(crate) fn from_existing(path: &Path) -> io::Result<Self> {
        let directory = Dir::open_ambient_dir(path, ambient_authority())?;
        Ok(Self {
            directory: Arc::new(directory),
        })
    }

    pub(crate) fn open_existing_no_follow(path: &Path) -> io::Result<Self> {
        if !path.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "local capability root must be absolute",
            ));
        }
        let (anchor, tail) = absolute_anchor_and_tail(path)?;
        let mut directory = Dir::open_ambient_dir(anchor, ambient_authority())?;
        for component in tail {
            directory = directory.open_dir_nofollow(&component)?;
        }
        Ok(Self {
            directory: Arc::new(directory),
        })
    }

    pub(crate) fn directory(&self) -> &Dir {
        self.directory.as_ref()
    }

    pub(crate) fn matches_existing_path(&self, path: &Path) -> io::Result<bool> {
        let reopened = Self::open_existing_no_follow(path)?;
        let retained = Handle::from_file(self.directory.try_clone()?.into_std_file())?;
        let reopened = Handle::from_file(reopened.directory.try_clone()?.into_std_file())?;
        Ok(retained == reopened)
    }

    pub(crate) fn create_dir_all(&self, relative: &Path) -> io::Result<()> {
        self.open_or_create_directory(relative).map(|_| ())
    }

    pub(crate) fn read_file(
        &self,
        relative: &Path,
        max_bytes: Option<usize>,
    ) -> Result<Option<Vec<u8>>, CapabilityWriteError> {
        self.reject_symlink_components(relative)?;
        let (parent_path, file_name) = split_parent(relative)?;
        let parent = self.open_existing_directory(parent_path)?;
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let mut file = parent.open_with(file_name, &options)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a file").into());
        }
        if max_bytes.is_some_and(|limit| metadata.len() > limit as u64) {
            return Ok(None);
        }
        let mut bytes = Vec::with_capacity(
            max_bytes
                .unwrap_or(metadata.len() as usize)
                .min(metadata.len() as usize),
        );
        match max_bytes {
            Some(limit) => {
                file.take((limit as u64).saturating_add(1))
                    .read_to_end(&mut bytes)?;
                if bytes.len() > limit {
                    return Ok(None);
                }
            }
            None => {
                file.read_to_end(&mut bytes)?;
            }
        }
        Ok(Some(bytes))
    }

    pub(crate) fn list_dir_bounded(
        &self,
        relative: &Path,
        max_entries: usize,
    ) -> Result<Vec<CapabilityDirectoryEntry>, CapabilityWriteError> {
        self.reject_symlink_components(relative)?;
        let directory = self.open_existing_directory(relative)?;
        let mut entries = Vec::new();
        for entry in directory.read_dir(".")? {
            if entries.len() >= max_entries {
                break;
            }
            let entry = entry?;
            entries.push(capability_directory_entry(entry)?);
        }
        Ok(entries)
    }

    pub(crate) fn list_dir_page(
        &self,
        relative: &Path,
        after: Option<&str>,
        max_entries: usize,
    ) -> Result<Vec<CapabilityDirectoryEntry>, CapabilityWriteError> {
        self.reject_symlink_components(relative)?;
        let directory = self.open_existing_directory(relative)?;
        let mut page = std::collections::BTreeMap::<String, CapabilityDirectoryEntry>::new();
        for entry in directory.read_dir(".")? {
            let entry = capability_directory_entry(entry?)?;
            let name = entry.name.to_string_lossy().to_string();
            if after.is_some_and(|cursor| name.as_str() <= cursor) {
                continue;
            }
            page.insert(name, entry);
            if page.len() > max_entries {
                page.pop_last();
            }
        }
        Ok(page.into_values().collect())
    }

    pub(crate) fn metadata(
        &self,
        relative: &Path,
    ) -> Result<CapabilityMetadata, CapabilityWriteError> {
        self.reject_symlink_components(relative)?;
        let metadata = if relative.as_os_str().is_empty() {
            self.directory.metadata(".")?
        } else {
            let (parent_path, file_name) = split_parent(relative)?;
            self.open_existing_directory(parent_path)?
                .symlink_metadata(file_name)?
        };
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            return Err(CapabilityWriteError::SymlinkEscape);
        }
        Ok(CapabilityMetadata {
            file_type: if file_type.is_file() {
                CapabilityFileType::File
            } else if file_type.is_dir() {
                CapabilityFileType::Directory
            } else {
                CapabilityFileType::Other
            },
            len: metadata.len(),
            modified: metadata.modified().ok().map(|time| time.into_std()),
        })
    }

    pub(crate) fn remove(&self, relative: &Path) -> Result<(), CapabilityWriteError> {
        self.reject_symlink_components(relative)?;
        let (parent_path, file_name) = split_parent(relative)?;
        let parent = self.open_existing_directory(parent_path)?;
        let metadata = parent.symlink_metadata(file_name)?;
        if metadata.file_type().is_symlink() {
            return Err(CapabilityWriteError::SymlinkEscape);
        }
        if metadata.is_dir() {
            parent.remove_dir_all(file_name)?;
        } else {
            parent.remove_file(file_name)?;
        }
        Ok(())
    }

    /// Creates and admits a descendant relative to this held directory.
    /// No ambient pathname is reopened and symlink components are rejected.
    pub fn create_dir_capability(&self, relative: &Path) -> io::Result<Self> {
        let directory = self.open_or_create_directory(relative)?;
        Ok(Self {
            directory: Arc::new(directory),
        })
    }

    pub(crate) fn append(&self, relative: &Path, bytes: &[u8]) -> Result<(), CapabilityWriteError> {
        self.reject_symlink_components(relative)?;
        let (parent, file_name) = split_parent(relative)?;
        let parent = self.open_or_create_directory(parent)?;
        let mut options = OpenOptions::new();
        options.write(true).append(true).create(true);
        options.follow(FollowSymlinks::No);
        let mut file = parent.open_with(file_name, &options)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    }

    pub(crate) fn atomic_write(
        &self,
        relative: &Path,
        bytes: &[u8],
        cas: CasExpectation,
        temp_counter: u64,
    ) -> Result<(), CapabilityWriteError> {
        self.reject_symlink_components(relative)?;
        let (parent_path, file_name) = split_parent(relative)?;
        let parent = self.open_or_create_directory(parent_path)?;
        if matches!(cas, CasExpectation::Absent) {
            match parent.symlink_metadata(file_name) {
                Ok(_) => return Err(CapabilityWriteError::VersionMismatch),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }

        let (temp_name, mut file) = create_unique_temp(&parent, file_name, temp_counter)?;
        if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
            let _ = parent.remove_file(&temp_name);
            return Err(error.into());
        }
        drop(file);

        match cas {
            CasExpectation::Any => {
                if let Err(error) = parent.rename(&temp_name, &parent, file_name) {
                    let _ = parent.remove_file(&temp_name);
                    return Err(error.into());
                }
            }
            CasExpectation::Absent => match parent.hard_link(&temp_name, &parent, file_name) {
                Ok(()) => {
                    if let Err(error) = parent.remove_file(&temp_name) {
                        tracing::debug!(reason = %error, "best-effort cleanup of published local write temp failed");
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    let _ = parent.remove_file(&temp_name);
                    return Err(CapabilityWriteError::VersionMismatch);
                }
                Err(error) => {
                    let _ = parent.remove_file(&temp_name);
                    return Err(error.into());
                }
            },
            CasExpectation::Version(_) => {
                let _ = parent.remove_file(&temp_name);
                return Err(CapabilityWriteError::UnsupportedVersion);
            }
        }
        sync_directory(&parent)?;
        Ok(())
    }

    pub(crate) fn create_subtree_atomic(
        &self,
        relative: &Path,
        entries: Vec<(std::path::PathBuf, Vec<u8>)>,
        temp_counter: u64,
    ) -> Result<(), CapabilityWriteError> {
        self.reject_symlink_components(relative)?;
        let (parent_path, target_name) = split_parent(relative)?;
        let parent = self.open_or_create_directory(parent_path)?;
        match parent.symlink_metadata(target_name) {
            Ok(_) => return Err(CapabilityWriteError::VersionMismatch),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }

        let staging_name = create_unique_subtree_dir(&parent, target_name, temp_counter)?;
        let staging = parent.open_dir_nofollow(&staging_name)?;
        let materialize = (|| -> io::Result<()> {
            for (relative_entry, bytes) in entries {
                let (entry_parent, entry_name) = split_parent(&relative_entry)?;
                let entry_parent = open_or_create_from(&staging, entry_parent)?;
                let mut options = OpenOptions::new();
                options.write(true).create_new(true);
                let mut file = entry_parent.open_with(entry_name, &options)?;
                file.write_all(&bytes)?;
                file.sync_all()?;
            }
            sync_directory(&staging)
        })();
        drop(staging);
        if let Err(error) = materialize {
            let _ = parent.remove_dir_all(&staging_name);
            return Err(error.into());
        }

        if let Err(error) = rename_subtree_absent(&parent, &staging_name, target_name) {
            let _ = parent.remove_dir_all(&staging_name);
            if error.kind() == io::ErrorKind::AlreadyExists {
                return Err(CapabilityWriteError::VersionMismatch);
            }
            return Err(error.into());
        }
        sync_directory(&parent)?;
        Ok(())
    }

    fn reject_symlink_components(&self, relative: &Path) -> Result<(), CapabilityWriteError> {
        let mut directory = self.directory.try_clone()?;
        let components = relative.components().collect::<Vec<_>>();
        for (index, component) in components.iter().enumerate() {
            let Component::Normal(name) = component else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "capability-relative path contains a non-normal component",
                )
                .into());
            };
            match directory.symlink_metadata(name) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(CapabilityWriteError::SymlinkEscape);
                }
                Ok(_) if index + 1 < components.len() => {
                    directory = directory.open_dir_nofollow(name)?;
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => break,
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    fn open_or_create_directory(&self, relative: &Path) -> io::Result<Dir> {
        let mut directory = self.directory.try_clone()?;
        for component in relative.components() {
            let Component::Normal(name) = component else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "capability-relative path contains a non-normal component",
                ));
            };
            directory = match directory.open_dir_nofollow(name) {
                Ok(child) => child,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    match directory.create_dir(name) {
                        Ok(()) => {}
                        Err(create_error)
                            if create_error.kind() == io::ErrorKind::AlreadyExists => {}
                        Err(create_error) => return Err(create_error),
                    }
                    directory.open_dir_nofollow(name)?
                }
                Err(error) => return Err(error),
            };
        }
        Ok(directory)
    }

    fn open_existing_directory(&self, relative: &Path) -> io::Result<Dir> {
        let mut directory = self.directory.try_clone()?;
        for component in relative.components() {
            let Component::Normal(name) = component else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "capability-relative path contains a non-normal component",
                ));
            };
            directory = directory.open_dir_nofollow(name)?;
        }
        Ok(directory)
    }
}

fn capability_directory_entry(
    entry: cap_std::fs::DirEntry,
) -> Result<CapabilityDirectoryEntry, CapabilityWriteError> {
    let file_type = entry.file_type()?;
    if file_type.is_symlink() {
        return Err(CapabilityWriteError::SymlinkEscape);
    }
    Ok(CapabilityDirectoryEntry {
        name: entry.file_name(),
        file_type: if file_type.is_file() {
            CapabilityFileType::File
        } else if file_type.is_dir() {
            CapabilityFileType::Directory
        } else {
            CapabilityFileType::Other
        },
    })
}

#[derive(Debug)]
pub(crate) enum CapabilityWriteError {
    Io(io::Error),
    SymlinkEscape,
    VersionMismatch,
    UnsupportedVersion,
}

impl From<io::Error> for CapabilityWriteError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

fn split_parent(relative: &Path) -> io::Result<(&Path, &std::ffi::OsStr)> {
    let file_name = relative.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "local write path has no file name",
        )
    })?;
    Ok((
        relative.parent().unwrap_or_else(|| Path::new("")),
        file_name,
    ))
}

const TEMP_CREATE_RETRIES: u32 = 16;

fn temporary_name(file_name: &std::ffi::OsStr, counter: u64, attempt: u32) -> OsString {
    let mut name = OsString::from(".");
    name.push(file_name);
    name.push(format!(
        ".ironclaw-{}-{counter}-{attempt}.tmp",
        std::process::id()
    ));
    name
}

fn create_unique_temp(
    parent: &Dir,
    file_name: &std::ffi::OsStr,
    counter: u64,
) -> io::Result<(OsString, cap_std::fs::File)> {
    for attempt in 0..TEMP_CREATE_RETRIES {
        let temp_name = temporary_name(file_name, counter, attempt);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        match parent.open_with(&temp_name, &options) {
            Ok(file) => return Ok((temp_name, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "local write exhausted temporary-name collision retries",
    ))
}

fn create_unique_subtree_dir(
    parent: &Dir,
    target_name: &std::ffi::OsStr,
    counter: u64,
) -> io::Result<OsString> {
    for attempt in 0..TEMP_CREATE_RETRIES {
        let mut name = OsString::from(".");
        name.push(target_name);
        name.push(format!(
            ".ironclaw-{}-{counter}-{attempt}.subtree.tmp",
            std::process::id()
        ));
        match parent.create_dir(&name) {
            Ok(()) => return Ok(name),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "local subtree exhausted temporary-name collision retries",
    ))
}

fn open_or_create_from(directory: &Dir, relative: &Path) -> io::Result<Dir> {
    let capability = DiskDirectoryCapability {
        directory: Arc::new(directory.try_clone()?),
    };
    capability.open_or_create_directory(relative)
}

fn absolute_anchor_and_tail(path: &Path) -> io::Result<(std::path::PathBuf, Vec<OsString>)> {
    let mut anchor = std::path::PathBuf::new();
    let mut tail = Vec::new();
    let mut rooted = false;
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => anchor.push(prefix.as_os_str()),
            Component::RootDir => {
                anchor.push(component.as_os_str());
                rooted = true;
            }
            Component::Normal(name) if rooted => tail.push(name.to_os_string()),
            Component::CurDir => {}
            Component::Normal(_) | Component::ParentDir => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "local capability root must be an absolute ordinary path",
                ));
            }
        }
    }
    if !rooted {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "local capability root must be absolute",
        ));
    }
    Ok((anchor, tail))
}

#[cfg(windows)]
fn sync_directory(_directory: &Dir) -> io::Result<()> {
    // cap-std intentionally opens Windows directories read-only and without
    // share-delete. FlushFileBuffers can reject such directory handles even
    // after publication succeeded; file contents were synced before rename.
    Ok(())
}

#[cfg(not(windows))]
fn sync_directory(directory: &Dir) -> io::Result<()> {
    // On Linux cap-std retains directory capabilities as traversal-only
    // `O_PATH` descriptors. Cloning and fsyncing that descriptor returns
    // EBADF after publication has already succeeded. Re-open `.` relative to
    // the retained capability with read access so the resulting descriptor is
    // syncable, while preserving the no-ambient-path containment boundary.
    let mut options = OpenOptions::new();
    options.read(true);
    options.follow(FollowSymlinks::No).maybe_dir(true);
    directory.open_with(".", &options)?.into_std().sync_all()
}

#[cfg(windows)]
fn rename_subtree_absent(
    parent: &Dir,
    from: &std::ffi::OsStr,
    to: &std::ffi::OsStr,
) -> io::Result<()> {
    // std/cap-std directory rename on Windows fails when the target exists.
    parent.rename(from, parent, to)
}

#[cfg(any(
    target_os = "android",
    target_os = "ios",
    target_os = "linux",
    target_os = "macos",
    target_os = "redox",
    target_os = "tvos",
    target_os = "visionos",
    target_os = "watchos"
))]
fn rename_subtree_absent(
    parent: &Dir,
    from: &std::ffi::OsStr,
    to: &std::ffi::OsStr,
) -> io::Result<()> {
    Ok(rustix::fs::renameat_with(
        parent,
        from,
        parent,
        to,
        rustix::fs::RenameFlags::NOREPLACE,
    )?)
}

#[cfg(not(any(
    windows,
    target_os = "android",
    target_os = "ios",
    target_os = "linux",
    target_os = "macos",
    target_os = "redox",
    target_os = "tvos",
    target_os = "visionos",
    target_os = "watchos"
)))]
fn rename_subtree_absent(
    _parent: &Dir,
    _from: &std::ffi::OsStr,
    _to: &std::ffi::OsStr,
) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic no-replace subtree publication is unsupported on this platform",
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    #[cfg(unix)]
    #[test]
    fn descendant_creation_stays_with_admitted_parent_after_path_replacement() {
        // macOS exposes `/var` as a symlink to `/private/var`; this primitive
        // deliberately rejects every symlink component. Use the ordinary
        // `/private/tmp` path so the fixture reaches the replacement race.
        #[cfg(target_os = "macos")]
        let temp = tempfile::Builder::new()
            .tempdir_in("/private/tmp")
            .expect("temporary root");
        #[cfg(not(target_os = "macos"))]
        let temp = tempfile::tempdir().expect("temporary root");
        let admitted_path = temp.path().join("system");
        let moved_path = temp.path().join("original-system");
        let outside = temp.path().join("outside");
        std::fs::create_dir(&admitted_path).expect("system root");
        std::fs::create_dir(&outside).expect("outside root");
        let admitted = super::DiskDirectoryCapability::admit_or_create(&admitted_path)
            .expect("admit system root");

        std::fs::rename(&admitted_path, &moved_path).expect("replace admitted pathname");
        std::os::unix::fs::symlink(&outside, &admitted_path).expect("replacement alias");

        admitted
            .create_dir_capability(std::path::Path::new("prompts/nested"))
            .expect("descriptor-relative descendant creation");
        assert!(moved_path.join("prompts/nested").is_dir());
        assert!(!outside.join("prompts").exists());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn blocking_descriptor_work_does_not_stall_the_async_executor() {
        let release = Arc::new(Barrier::new(2));
        let worker_release = Arc::clone(&release);
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(super::run_capability_blocking(move || {
            let _ = entered_tx.send(());
            worker_release.wait();
            Ok::<_, std::io::Error>(())
        }));

        entered_rx.await.unwrap();
        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            tokio::time::sleep(std::time::Duration::from_millis(1)),
        )
        .await
        .expect("descriptor work must run outside the async executor");
        release.wait();
        task.await.unwrap().unwrap();
    }

    #[test]
    fn atomic_write_retries_a_preexisting_temp_name_without_losing_the_write() {
        use crate::CasExpectation;

        let storage = tempfile::tempdir().unwrap();
        let storage_root = storage.path().canonicalize().unwrap();
        let capability = super::DiskDirectoryCapability::admit_or_create(&storage_root).unwrap();
        let first_temp = super::temporary_name(std::ffi::OsStr::new("result.txt"), 7, 0);
        std::fs::write(storage_root.join(first_temp), b"another writer").unwrap();

        capability
            .atomic_write(
                std::path::Path::new("result.txt"),
                b"our write",
                CasExpectation::Any,
                7,
            )
            .unwrap();

        assert_eq!(
            std::fs::read(storage_root.join("result.txt")).unwrap(),
            b"our write"
        );
    }
}
