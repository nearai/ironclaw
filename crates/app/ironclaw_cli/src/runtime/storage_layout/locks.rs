//! Advisory locks that serialize the boot-time layout migration.

use super::filesystem::*;
use super::*;

pub(super) struct MigrationLock {
    #[cfg(any(unix, windows))]
    _file: File,
}

pub(super) fn acquire_named_lock(
    directory: &Path,
    file_name: &str,
    operation: &str,
) -> anyhow::Result<MigrationLock> {
    #[cfg(not(any(unix, windows)))]
    {
        bail!(
            "descriptor-backed advisory locks are unsupported on this platform; refusing {operation} at {}",
            directory.display()
        );
    }

    #[cfg(any(unix, windows))]
    {
        let path = directory.join(file_name);
        require_ordinary_directory(directory)?;
        let mut file = open_migration_lock_file(&path)?;
        fs4::FileExt::try_lock(&file).with_context(|| {
        format!(
            "another {operation} is holding advisory lock {}; wait for it to finish before retrying",
            path.display()
        )
    })?;
        file.set_len(0)
            .with_context(|| format!("clear advisory lock {}", path.display()))?;
        writeln!(file, "pid={}", std::process::id())
            .with_context(|| format!("write advisory lock {}", path.display()))?;
        file.sync_all()
            .with_context(|| format!("sync advisory lock {}", path.display()))?;
        sync_directory(directory)?;
        Ok(MigrationLock { _file: file })
    }
}

#[cfg(unix)]
pub(super) fn open_migration_lock_file(path: &Path) -> anyhow::Result<File> {
    if path.exists() {
        require_ordinary_file(path)?;
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path).with_context(|| {
        format!(
            "open advisory lock without following links {}",
            path.display()
        )
    })?;
    if !file
        .metadata()
        .with_context(|| format!("inspect opened advisory lock {}", path.display()))?
        .is_file()
    {
        bail!("expected an ordinary file at {}", path.display());
    }
    Ok(file)
}

#[cfg(windows)]
pub(super) fn open_migration_lock_file(path: &Path) -> anyhow::Result<File> {
    use std::os::windows::fs::{MetadataExt as _, OpenOptionsExt as _};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
    };

    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    let file = options.open(path).with_context(|| {
        format!(
            "open advisory lock without following links {}",
            path.display()
        )
    })?;
    let metadata = file
        .metadata()
        .with_context(|| format!("inspect opened advisory lock {}", path.display()))?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 || !metadata.is_file() {
        bail!(
            "expected an ordinary non-reparse-point file at {}",
            path.display()
        );
    }
    Ok(file)
}
