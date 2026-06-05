//! Google Drive WASM Tool for IronClaw.
//!
//! Provides Google Drive integration for searching, accessing, uploading,
//! sharing, and organizing files and folders. Supports both personal and
//! shared (organizational) drives.
//!
//! # Capabilities Required
//!
//! - HTTP: `www.googleapis.com/drive/v3/*` and `www.googleapis.com/upload/drive/v3/*`
//! - Credentials: staged Google product-auth account token injected by the host.
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

use types::{GoogleDriveAction, ToolContext};

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../../../../wit/tool.wit",
});

struct GoogleDriveTool;

impl exports::near::agent::tool::Guest for GoogleDriveTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params, req.context.as_deref()) {
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
        // Derived from `GoogleDriveAction` via `schemars::JsonSchema` so the
        // advertised schema can never drift from the serde contract. Each
        // enum variant becomes a `oneOf` entry with its own `required`
        // array — the agent sees that `file_id` is required when
        // `action == "get_file"`, etc.
        let schema = schemars::schema_for!(types::GoogleDriveAction);
        serde_json::to_string(&schema).unwrap_or_else(|_| "{}".to_string())
    }

    fn description() -> String {
        "Google Drive integration for searching, accessing, uploading, sharing, and organizing \
         files and folders. Supports personal drives and shared (organizational) drives via the \
         corpora parameter. Can search with Drive query syntax, download text files, upload new \
         files, manage folder structure, and control sharing permissions. The host injects a \
         Google product-auth credential with the Drive scope. \
         To discover all available API operations, use http GET to fetch \
         <https://www.googleapis.com/discovery/v1/apis/drive/v3/rest> (public, no auth needed)."
            .to_string()
    }
}

fn execute_inner(params: &str, context: Option<&str>) -> Result<String, String> {
    let action_name = action_from_context(context)?;
    let params = params_with_action(params, action_name)?;
    let action: GoogleDriveAction =
        serde_json::from_value(params).map_err(|e| format!("Invalid parameters: {}", e))?;

    crate::near::agent::host::log(
        crate::near::agent::host::LogLevel::Debug,
        &format!("Executing Google Drive action: {action_name}"),
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

fn action_from_context(context: Option<&str>) -> Result<&'static str, String> {
    let context = context.ok_or_else(|| "missing_invocation_context".to_string())?;
    let context: ToolContext =
        serde_json::from_str(context).map_err(|_| "invalid_invocation_context".to_string())?;
    match context.capability_id.as_str() {
        "google-drive.list_files" => Ok("list_files"),
        "google-drive.get_file" => Ok("get_file"),
        "google-drive.download_file" => Ok("download_file"),
        "google-drive.upload_file" => Ok("upload_file"),
        "google-drive.update_file" => Ok("update_file"),
        "google-drive.create_folder" => Ok("create_folder"),
        "google-drive.delete_file" => Ok("delete_file"),
        "google-drive.trash_file" => Ok("trash_file"),
        "google-drive.share_file" => Ok("share_file"),
        "google-drive.list_permissions" => Ok("list_permissions"),
        "google-drive.remove_permission" => Ok("remove_permission"),
        "google-drive.list_shared_drives" => Ok("list_shared_drives"),
        _ => Err("unsupported_google_drive_capability".to_string()),
    }
}

fn params_with_action(params: &str, action: &str) -> Result<serde_json::Value, String> {
    let mut params: serde_json::Value = if params.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(params).map_err(|_| "invalid_parameters".to_string())?
    };
    let obj = params
        .as_object_mut()
        .ok_or_else(|| "invalid_parameters".to_string())?;
    if obj.contains_key("action") {
        return Err("invalid_parameters".to_string());
    }
    obj.insert(
        "action".to_string(),
        serde_json::Value::String(action.to_string()),
    );
    Ok(params)
}

export!(GoogleDriveTool);
