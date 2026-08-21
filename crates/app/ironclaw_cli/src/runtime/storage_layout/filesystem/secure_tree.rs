use super::*;

pub(in super::super) fn validate_master_key_source(path: &Path) -> anyhow::Result<File> {
    require_ordinary_file(path)?;
    let file = open_file_no_follow(path)?;
    verify_master_key_policy(&file, path, "source")?;
    Ok(file)
}

pub(in super::super) fn verify_master_key_policy(
    file: &File,
    path: &Path,
    location: &str,
) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let mode = file
            .metadata()
            .with_context(|| format!("read {location} master key metadata at {}", path.display()))?
            .mode()
            & 0o777;
        if mode & 0o077 != 0 {
            bail!(
                "{location} master key at {} must not grant group or world permissions; found mode {mode:03o}",
                path.display()
            );
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (file, path, location);
    }
    Ok(())
}

pub(in super::super) fn open_file_no_follow(path: &Path) -> anyhow::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path).with_context(|| {
        format!(
            "open ordinary source file without following links {}",
            path.display()
        )
    })
}

pub(in super::super) fn open_directory_no_follow(path: &Path) -> anyhow::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW);
    }
    options.open(path).with_context(|| {
        format!(
            "open ordinary directory without following links {}",
            path.display()
        )
    })
}

pub(in super::super) fn read_utf8_file_no_follow(path: &Path) -> anyhow::Result<String> {
    require_ordinary_file(path)?;
    let mut file = open_file_no_follow(path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .with_context(|| format!("read UTF-8 text file {}", path.display()))?;
    Ok(contents)
}

pub(in super::super) fn validate_ordinary_tree(path: &Path) -> anyhow::Result<()> {
    inspect_ordinary_tree(path).map(|_| ())
}

pub(in super::super) fn require_ordinary_file(path: &Path) -> anyhow::Result<()> {
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("inspect file {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "expected an ordinary non-symlink file at {}",
            path.display()
        );
    }
    Ok(())
}

pub(in super::super) fn require_ordinary_directory(path: &Path) -> anyhow::Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect directory {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "expected an ordinary non-symlink directory at {}",
            path.display()
        );
    }
    let handle = open_directory_no_follow(path)?;
    if !handle
        .metadata()
        .with_context(|| format!("read opened directory metadata {}", path.display()))?
        .is_dir()
    {
        bail!(
            "expected an ordinary non-symlink directory at {}",
            path.display()
        );
    }
    Ok(())
}

pub(in super::super) fn directory_is_empty(path: &Path) -> anyhow::Result<bool> {
    require_ordinary_directory(path)?;
    Ok(fs::read_dir(path)
        .with_context(|| format!("read directory {}", path.display()))?
        .next()
        .is_none())
}

pub(in super::super) fn directory_has_content(path: &Path) -> anyhow::Result<bool> {
    inspect_ordinary_tree(path)
}

fn inspect_ordinary_tree(path: &Path) -> anyhow::Result<bool> {
    ironclaw_filesystem::inspect_ordinary_host_tree(
        &ironclaw_host_api::path::HostPath::from_path_buf(path.to_path_buf()),
    )
    .with_context(|| format!("validate ordinary adoption source tree {}", path.display()))
}

pub(in super::super) fn sync_directory(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        open_directory_no_follow(path)?
            .sync_all()
            .with_context(|| format!("sync directory {}", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
