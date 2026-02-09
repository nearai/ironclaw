//! Types for Google Drive API requests and responses.

use serde::{Deserialize, Serialize};

/// Input parameters for the Google Drive tool.
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum GoogleDriveAction {
    /// Search/list files and folders.
    ListFiles {
        /// Drive search query (same syntax as Drive search).
        /// Examples: "name contains 'report'", "mimeType = 'application/pdf'",
        /// "'folderId' in parents", "sharedWithMe = true".
        #[serde(default)]
        query: Option<String>,
        /// Maximum number of results (default: 25, max: 1000).
        #[serde(default = "default_page_size")]
        page_size: u32,
        /// Sort order (e.g., "modifiedTime desc", "name").
        #[serde(default)]
        order_by: Option<String>,
        /// Search corpus: "user" (personal, default), "drive" (specific shared drive),
        /// "domain" (org-wide), "allDrives" (everything accessible).
        #[serde(default = "default_corpora")]
        corpora: String,
        /// Shared drive ID (required when corpora is "drive").
        #[serde(default)]
        drive_id: Option<String>,
        /// Page token for pagination.
        #[serde(default)]
        page_token: Option<String>,
    },

    /// Get file metadata.
    GetFile {
        /// The file ID.
        file_id: String,
    },

    /// Download file content as text.
    /// Only works for text-based files. For Google Docs/Sheets/Slides,
    /// exports as plain text / CSV / plain text respectively.
    DownloadFile {
        /// The file ID.
        file_id: String,
        /// Export MIME type for Google Workspace files.
        /// Defaults: Docs -> "text/plain", Sheets -> "text/csv",
        /// Slides -> "text/plain", Drawings -> "image/svg+xml".
        #[serde(default)]
        export_mime_type: Option<String>,
    },

    /// Upload a new file (text content).
    UploadFile {
        /// File name.
        name: String,
        /// File content (text).
        content: String,
        /// MIME type (default: "text/plain").
        #[serde(default = "default_mime_type")]
        mime_type: String,
        /// Parent folder ID. Omit for root.
        #[serde(default)]
        parent_id: Option<String>,
        /// File description.
        #[serde(default)]
        description: Option<String>,
    },

    /// Update file metadata (rename, move, change description).
    UpdateFile {
        /// The file ID.
        file_id: String,
        /// New file name.
        #[serde(default)]
        name: Option<String>,
        /// New description.
        #[serde(default)]
        description: Option<String>,
        /// Move to this parent folder (removes from current parents).
        #[serde(default)]
        move_to_parent: Option<String>,
        /// Star or unstar the file.
        #[serde(default)]
        starred: Option<bool>,
    },

    /// Create a folder.
    CreateFolder {
        /// Folder name.
        name: String,
        /// Parent folder ID. Omit for root.
        #[serde(default)]
        parent_id: Option<String>,
        /// Folder description.
        #[serde(default)]
        description: Option<String>,
    },

    /// Delete a file or folder (permanent).
    DeleteFile {
        /// The file ID to delete.
        file_id: String,
    },

    /// Move a file to trash.
    TrashFile {
        /// The file ID to trash.
        file_id: String,
    },

    /// Share a file or folder with someone.
    ShareFile {
        /// The file ID to share.
        file_id: String,
        /// Recipient email address.
        email: String,
        /// Permission role: "reader", "commenter", "writer", "organizer".
        #[serde(default = "default_role")]
        role: String,
        /// Optional message to include in the sharing notification.
        #[serde(default)]
        message: Option<String>,
    },

    /// List who a file is shared with.
    ListPermissions {
        /// The file ID.
        file_id: String,
    },

    /// Remove sharing (revoke a permission).
    RemovePermission {
        /// The file ID.
        file_id: String,
        /// The permission ID to remove.
        permission_id: String,
    },

    /// List shared drives the user has access to.
    ListSharedDrives {
        /// Maximum results (default: 25).
        #[serde(default = "default_page_size")]
        page_size: u32,
    },
}

fn default_page_size() -> u32 {
    25
}

fn default_corpora() -> String {
    "user".to_string()
}

fn default_mime_type() -> String {
    "text/plain".to_string()
}

fn default_role() -> String {
    "reader".to_string()
}

/// A Google Drive file or folder.
#[derive(Debug, Serialize)]
pub struct DriveFile {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_view_link: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub parents: Vec<String>,
    pub shared: bool,
    pub starred: bool,
    pub trashed: bool,
    pub owned_by_me: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drive_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub owners: Vec<Owner>,
    pub is_folder: bool,
}

/// File owner info.
#[derive(Debug, Serialize)]
pub struct Owner {
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// A sharing permission.
#[derive(Debug, Serialize)]
pub struct Permission {
    pub id: String,
    pub role: String,
    #[serde(rename = "type")]
    pub permission_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// A shared drive.
#[derive(Debug, Serialize)]
pub struct SharedDrive {
    pub id: String,
    pub name: String,
}

/// Result from list_files.
#[derive(Debug, Serialize)]
pub struct ListFilesResult {
    pub files: Vec<DriveFile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_page_token: Option<String>,
}

/// Result from get_file or upload/update.
#[derive(Debug, Serialize)]
pub struct FileResult {
    pub file: DriveFile,
}

/// Result from download_file.
#[derive(Debug, Serialize)]
pub struct DownloadResult {
    pub file_id: String,
    pub name: String,
    pub mime_type: String,
    pub content: String,
}

/// Result from delete/trash.
#[derive(Debug, Serialize)]
pub struct DeleteResult {
    pub file_id: String,
    pub deleted: bool,
}

/// Result from share_file.
#[derive(Debug, Serialize)]
pub struct ShareResult {
    pub permission_id: String,
    pub role: String,
    pub email: String,
}

/// Result from list_permissions.
#[derive(Debug, Serialize)]
pub struct ListPermissionsResult {
    pub permissions: Vec<Permission>,
}

/// Result from list_shared_drives.
#[derive(Debug, Serialize)]
pub struct ListSharedDrivesResult {
    pub drives: Vec<SharedDrive>,
}
