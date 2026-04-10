//! Types for Google Drive API requests and responses.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Input parameters for the Google Drive tool.
///
/// `JsonSchema` is derived so the advertised tool schema mirrors the
/// serde-enforced contract: each variant becomes a `oneOf` entry with
/// its own `required` array, so the agent knows which fields apply to
/// which `action`. Hand-writing the schema previously declared every
/// per-variant field as top-level optional, leading to runtime
/// `missing field 'file_id'` errors when the LLM omitted them.
#[derive(Debug, Deserialize, JsonSchema)]
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole motivation for the schemars derive: when the agent calls
    /// `{"action": "get_file"}` without `file_id`, serde must reject it
    /// (matching the schema, which now also requires file_id under that
    /// variant). Previously the schema said "file_id is optional" while
    /// the code rejected it, so the agent kept making malformed calls.
    #[test]
    fn get_file_requires_file_id_at_serde_layer() {
        let bad: Result<GoogleDriveAction, _> = serde_json::from_str(r#"{"action":"get_file"}"#);
        assert!(
            bad.is_err(),
            "serde must reject get_file without file_id"
        );
        let good: Result<GoogleDriveAction, _> =
            serde_json::from_str(r#"{"action":"get_file","file_id":"abc123"}"#);
        assert!(good.is_ok(), "serde must accept get_file with file_id");
    }

    /// The schema must reflect the same requirement so the agent can see
    /// it before constructing a call. Each enum variant should appear as
    /// a `oneOf` entry with `file_id` in `required` when applicable.
    #[test]
    fn schema_marks_file_id_required_for_get_file() {
        let schema = schemars::schema_for!(GoogleDriveAction);
        let json = serde_json::to_value(&schema).unwrap();

        let one_of = json
            .get("oneOf")
            .and_then(|v| v.as_array())
            .expect("schemars should emit a oneOf for tagged enum");
        assert!(
            one_of.len() >= 12,
            "should have one oneOf entry per action (got {})",
            one_of.len()
        );

        // Find the get_file branch and check its required array.
        let get_file_branch = one_of
            .iter()
            .find(|entry| {
                entry
                    .get("properties")
                    .and_then(|p| p.get("action"))
                    .and_then(|a| a.get("const"))
                    .and_then(|c| c.as_str())
                    == Some("get_file")
            })
            .expect("schema should contain a get_file branch");

        let required: Vec<&str> = get_file_branch
            .get("required")
            .and_then(|r| r.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        assert!(
            required.contains(&"file_id"),
            "get_file branch should require file_id, got required={:?}",
            required
        );
        assert!(
            required.contains(&"action"),
            "get_file branch should require action, got required={:?}",
            required
        );
    }

    /// list_files takes only optional fields — `file_id` must NOT appear
    /// in its required array, even though it's listed in other variants.
    #[test]
    fn schema_does_not_require_file_id_for_list_files() {
        let schema = schemars::schema_for!(GoogleDriveAction);
        let json = serde_json::to_value(&schema).unwrap();
        let one_of = json["oneOf"].as_array().unwrap();

        let list_files_branch = one_of
            .iter()
            .find(|entry| {
                entry
                    .get("properties")
                    .and_then(|p| p.get("action"))
                    .and_then(|a| a.get("const"))
                    .and_then(|c| c.as_str())
                    == Some("list_files")
            })
            .expect("schema should contain a list_files branch");

        let required: Vec<&str> = list_files_branch
            .get("required")
            .and_then(|r| r.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        assert!(
            !required.contains(&"file_id"),
            "list_files must not require file_id (it has none)"
        );
        assert_eq!(
            required, ["action"],
            "list_files should require only the discriminator"
        );
    }
}
