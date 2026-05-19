use std::path::Path;

use ironclaw_filesystem::{FilesystemError, LocalFilesystem};
use ironclaw_host_api::{HostPath, VirtualPath};

pub fn local_filesystem(root: &Path) -> Result<LocalFilesystem, FilesystemError> {
    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/engine").expect("valid test virtual path"),
        HostPath::from_path_buf(root.to_path_buf()),
    )?;
    Ok(fs)
}
