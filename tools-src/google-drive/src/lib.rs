//! Google Drive WASM Tool for IronClaw.
//!
//! Provides Google Drive integration for searching, accessing, uploading,
//! sharing, and organizing files and folders. Supports both personal and
//! shared (organizational) drives.
//!
//! # Capabilities Required
//!
//! - HTTP: `www.googleapis.com/drive/v3/*` and `www.googleapis.com/upload/drive/v3/*`
//! - Secrets: `google_oauth_token` (shared OAuth 2.0 token, injected automatically)
//!
//! # Supported Actions
//!
//! - `list_files`: Search/list files with Drive query syntax and corpora selection
//! - `get_file`: Get file metadata
//! - `download_file`: Download file content as text (exports Google Docs/Sheets)
//! - `upload_file`: Upload a text file (multipart)
//! - `update_file`: Rename, move, star, or update description
//! - `create_folder`: Create a new folder
//! - `delete_file`: Permanently delete a file
//! - `trash_file`: Move to trash
//! - `share_file`: Share with a user (reader, commenter, writer, organizer)
//! - `list_permissions`: See who has access
//! - `remove_permission`: Revoke access
//! - `list_shared_drives`: List organizational shared drives
//!
//! # Example Usage
//!
//! ```json
//! {"action": "list_files", "query": "name contains 'report' and mimeType = 'application/pdf'"}
//! {"action": "list_files", "corpora": "drive", "drive_id": "0ABcd...", "query": "trashed = false"}
//! {"action": "share_file", "file_id": "abc123", "email": "alice@company.com", "role": "writer"}
//! ```

mod api;
mod types;

use types::GoogleDriveAction;

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

struct GoogleDriveTool;

impl exports::near::agent::tool::Guest for GoogleDriveTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params) {
            Ok(result) => exports::near::agent::tool::Response {
                output: Some(result),
                error: None,
            },
            Err(e) => exports::near::agent::tool::Response {
                output: None,
                error: Some(e),
            },
        }
    }

    fn schema() -> String {
        r#"{
            "type": "object",
            "required": ["action"],
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list_files", "get_file", "download_file", "upload_file", "update_file", "create_folder", "delete_file", "trash_file", "share_file", "list_permissions", "remove_permission", "list_shared_drives"],
                    "description": "The Google Drive operation to perform"
                },
                "file_id": {
                    "type": "string",
                    "description": "File ID. Required for: get_file, download_file, update_file, delete_file, trash_file, share_file, list_permissions, remove_permission"
                },
                "query": {
                    "type": "string",
                    "description": "Drive search query (e.g., \"name contains 'report'\", \"mimeType = 'application/pdf'\"). Used by: list_files"
                },
                "page_size": {
                    "type": "integer",
                    "description": "Max results (default: 25, max: 1000). Used by: list_files, list_shared_drives",
                    "default": 25
                },
                "order_by": {
                    "type": "string",
                    "description": "Sort order (e.g., 'modifiedTime desc', 'name'). Used by: list_files"
                },
                "corpora": {
                    "type": "string",
                    "enum": ["user", "drive", "domain", "allDrives"],
                    "description": "Search scope: 'user' (default), 'drive' (shared drive), 'domain', 'allDrives'. Used by: list_files",
                    "default": "user"
                },
                "drive_id": {
                    "type": "string",
                    "description": "Shared drive ID (required when corpora is 'drive'). Used by: list_files"
                },
                "page_token": {
                    "type": "string",
                    "description": "Token for next page of results. Used by: list_files"
                },
                "export_mime_type": {
                    "type": "string",
                    "description": "Export format for Google Workspace files (e.g., 'text/plain', 'text/csv'). Used by: download_file"
                },
                "name": {
                    "type": "string",
                    "description": "File/folder name. Required for: upload_file, create_folder. Optional for: update_file"
                },
                "content": {
                    "type": "string",
                    "description": "File content (text). Required for: upload_file"
                },
                "mime_type": {
                    "type": "string",
                    "description": "MIME type (default: 'text/plain'). Used by: upload_file",
                    "default": "text/plain"
                },
                "parent_id": {
                    "type": "string",
                    "description": "Parent folder ID (omit for root). Used by: upload_file, create_folder"
                },
                "description": {
                    "type": "string",
                    "description": "File/folder description. Used by: upload_file, update_file, create_folder"
                },
                "move_to_parent": {
                    "type": "string",
                    "description": "Move file to this folder ID. Used by: update_file"
                },
                "starred": {
                    "type": "boolean",
                    "description": "Star or unstar the file. Used by: update_file"
                },
                "email": {
                    "type": "string",
                    "description": "Recipient email address. Required for: share_file"
                },
                "role": {
                    "type": "string",
                    "enum": ["reader", "commenter", "writer", "organizer"],
                    "description": "Permission level (default: 'reader'). Used by: share_file",
                    "default": "reader"
                },
                "message": {
                    "type": "string",
                    "description": "Optional message in sharing notification. Used by: share_file"
                },
                "permission_id": {
                    "type": "string",
                    "description": "Permission ID to remove (from list_permissions). Required for: remove_permission"
                }
            }
        }"#
        .to_string()
    }

    fn description() -> String {
        "Google Drive integration for searching, accessing, uploading, sharing, and organizing \
         files and folders. Supports personal drives and shared (organizational) drives via the \
         corpora parameter. Can search with Drive query syntax, download text files, upload new \
         files, manage folder structure, and control sharing permissions. Requires a Google OAuth \
         token with the drive scope."
            .to_string()
    }
}

fn execute_inner(params: &str) -> Result<String, String> {
    if !crate::near::agent::host::secret_exists("google_oauth_token") {
        return Err(
            "Google OAuth token not configured. Run `ironclaw tool auth google-drive` to set up \
             OAuth, or set the GOOGLE_OAUTH_TOKEN environment variable."
                .to_string(),
        );
    }

    let action: GoogleDriveAction =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {}", e))?;

    crate::near::agent::host::log(
        crate::near::agent::host::LogLevel::Info,
        &format!("Executing Google Drive action: {:?}", action),
    );

    let result = match action {
        GoogleDriveAction::ListFiles {
            query,
            page_size,
            order_by,
            corpora,
            drive_id,
            page_token,
        } => {
            let result = api::list_files(
                query.as_deref(),
                page_size,
                order_by.as_deref(),
                &corpora,
                drive_id.as_deref(),
                page_token.as_deref(),
            )?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleDriveAction::GetFile { file_id } => {
            let result = api::get_file(&file_id)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleDriveAction::DownloadFile {
            file_id,
            export_mime_type,
        } => {
            let result = api::download_file(&file_id, export_mime_type.as_deref())?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleDriveAction::UploadFile {
            name,
            content,
            mime_type,
            parent_id,
            description,
        } => {
            let result = api::upload_file(
                &name,
                &content,
                &mime_type,
                parent_id.as_deref(),
                description.as_deref(),
            )?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleDriveAction::UpdateFile {
            file_id,
            name,
            description,
            move_to_parent,
            starred,
        } => {
            let result = api::update_file(
                &file_id,
                name.as_deref(),
                description.as_deref(),
                move_to_parent.as_deref(),
                starred,
            )?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleDriveAction::CreateFolder {
            name,
            parent_id,
            description,
        } => {
            let result = api::create_folder(&name, parent_id.as_deref(), description.as_deref())?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleDriveAction::DeleteFile { file_id } => {
            let result = api::delete_file(&file_id)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleDriveAction::TrashFile { file_id } => {
            let result = api::trash_file(&file_id)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleDriveAction::ShareFile {
            file_id,
            email,
            role,
            message,
        } => {
            let result = api::share_file(&file_id, &email, &role, message.as_deref())?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleDriveAction::ListPermissions { file_id } => {
            let result = api::list_permissions(&file_id)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleDriveAction::RemovePermission {
            file_id,
            permission_id,
        } => {
            let result = api::remove_permission(&file_id, &permission_id)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleDriveAction::ListSharedDrives { page_size } => {
            let result = api::list_shared_drives(page_size)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }
    };

    Ok(result)
}

export!(GoogleDriveTool);
