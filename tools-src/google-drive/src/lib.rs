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
            "oneOf": [
                {
                    "properties": {
                        "action": { "const": "list_files" },
                        "query": {
                            "type": "string",
                            "description": "Drive search query. Examples: \"name contains 'report'\", \"mimeType = 'application/pdf'\", \"'folderId' in parents\", \"sharedWithMe = true\""
                        },
                        "page_size": {
                            "type": "integer",
                            "description": "Max results (default: 25, max: 1000)",
                            "default": 25
                        },
                        "order_by": {
                            "type": "string",
                            "description": "Sort order (e.g., 'modifiedTime desc', 'name')"
                        },
                        "corpora": {
                            "type": "string",
                            "enum": ["user", "drive", "domain", "allDrives"],
                            "description": "Search scope: 'user' (personal, default), 'drive' (specific shared drive), 'domain' (org-wide), 'allDrives' (everything)",
                            "default": "user"
                        },
                        "drive_id": {
                            "type": "string",
                            "description": "Shared drive ID (required when corpora is 'drive')"
                        },
                        "page_token": {
                            "type": "string",
                            "description": "Token for next page of results"
                        }
                    },
                    "required": ["action"]
                },
                {
                    "properties": {
                        "action": { "const": "get_file" },
                        "file_id": {
                            "type": "string",
                            "description": "The file ID"
                        }
                    },
                    "required": ["action", "file_id"]
                },
                {
                    "properties": {
                        "action": { "const": "download_file" },
                        "file_id": {
                            "type": "string",
                            "description": "The file ID to download"
                        },
                        "export_mime_type": {
                            "type": "string",
                            "description": "Export format for Google Workspace files (e.g., 'text/plain', 'text/csv', 'application/pdf')"
                        }
                    },
                    "required": ["action", "file_id"]
                },
                {
                    "properties": {
                        "action": { "const": "upload_file" },
                        "name": {
                            "type": "string",
                            "description": "File name"
                        },
                        "content": {
                            "type": "string",
                            "description": "File content (text)"
                        },
                        "mime_type": {
                            "type": "string",
                            "description": "MIME type (default: 'text/plain')",
                            "default": "text/plain"
                        },
                        "parent_id": {
                            "type": "string",
                            "description": "Parent folder ID (omit for root)"
                        },
                        "description": {
                            "type": "string",
                            "description": "File description"
                        }
                    },
                    "required": ["action", "name", "content"]
                },
                {
                    "properties": {
                        "action": { "const": "update_file" },
                        "file_id": {
                            "type": "string",
                            "description": "The file ID to update"
                        },
                        "name": {
                            "type": "string",
                            "description": "New file name"
                        },
                        "description": {
                            "type": "string",
                            "description": "New description"
                        },
                        "move_to_parent": {
                            "type": "string",
                            "description": "Move file to this folder ID"
                        },
                        "starred": {
                            "type": "boolean",
                            "description": "Star or unstar the file"
                        }
                    },
                    "required": ["action", "file_id"]
                },
                {
                    "properties": {
                        "action": { "const": "create_folder" },
                        "name": {
                            "type": "string",
                            "description": "Folder name"
                        },
                        "parent_id": {
                            "type": "string",
                            "description": "Parent folder ID (omit for root)"
                        },
                        "description": {
                            "type": "string",
                            "description": "Folder description"
                        }
                    },
                    "required": ["action", "name"]
                },
                {
                    "properties": {
                        "action": { "const": "delete_file" },
                        "file_id": {
                            "type": "string",
                            "description": "The file ID to permanently delete"
                        }
                    },
                    "required": ["action", "file_id"]
                },
                {
                    "properties": {
                        "action": { "const": "trash_file" },
                        "file_id": {
                            "type": "string",
                            "description": "The file ID to move to trash"
                        }
                    },
                    "required": ["action", "file_id"]
                },
                {
                    "properties": {
                        "action": { "const": "share_file" },
                        "file_id": {
                            "type": "string",
                            "description": "The file ID to share"
                        },
                        "email": {
                            "type": "string",
                            "description": "Recipient email address"
                        },
                        "role": {
                            "type": "string",
                            "enum": ["reader", "commenter", "writer", "organizer"],
                            "description": "Permission level (default: 'reader')",
                            "default": "reader"
                        },
                        "message": {
                            "type": "string",
                            "description": "Optional message in sharing notification"
                        }
                    },
                    "required": ["action", "file_id", "email"]
                },
                {
                    "properties": {
                        "action": { "const": "list_permissions" },
                        "file_id": {
                            "type": "string",
                            "description": "The file ID to check permissions for"
                        }
                    },
                    "required": ["action", "file_id"]
                },
                {
                    "properties": {
                        "action": { "const": "remove_permission" },
                        "file_id": {
                            "type": "string",
                            "description": "The file ID"
                        },
                        "permission_id": {
                            "type": "string",
                            "description": "The permission ID to remove (get from list_permissions)"
                        }
                    },
                    "required": ["action", "file_id", "permission_id"]
                },
                {
                    "properties": {
                        "action": { "const": "list_shared_drives" },
                        "page_size": {
                            "type": "integer",
                            "description": "Max results (default: 25)",
                            "default": 25
                        }
                    },
                    "required": ["action"]
                }
            ]
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
