//! Google Drive API v3 implementation.
//!
//! All API calls go through the host's HTTP capability, which handles
//! credential injection and rate limiting. The WASM tool never sees
//! the actual OAuth token.

use crate::near::agent::host;
use crate::types::*;

const DRIVE_API_BASE: &str = "https://www.googleapis.com/drive/v3";
const UPLOAD_API_BASE: &str = "https://www.googleapis.com/upload/drive/v3";

/// Standard fields to request for file metadata.
const FILE_FIELDS: &str = "id,name,mimeType,description,size,createdTime,modifiedTime,\
    webViewLink,parents,shared,starred,trashed,ownedByMe,driveId,\
    owners(emailAddress,displayName)";

/// Make a Drive API call.
fn api_call(method: &str, path: &str, body: Option<&str>) -> Result<String, String> {
    let url = format!("{}/{}", DRIVE_API_BASE, path);

    let headers = if body.is_some() {
        r#"{"Content-Type": "application/json"}"#
    } else {
        "{}"
    };

    let body_bytes = body.map(|b| b.as_bytes().to_vec());

    host::log(
        host::LogLevel::Debug,
        &format!("Drive API: {} {}", method, path),
    );

    let response = host::http_request(method, &url, headers, body_bytes.as_deref(), None)?;

    if response.status < 200 || response.status >= 300 {
        let body_text = String::from_utf8_lossy(&response.body);
        return Err(format!(
            "Drive API returned status {}: {}",
            response.status, body_text
        ));
    }

    if response.body.is_empty() {
        return Ok(String::new());
    }

    String::from_utf8(response.body).map_err(|e| format!("Invalid UTF-8 in response: {}", e))
}

/// Make a raw API call that returns bytes (for file downloads).
fn api_call_raw(method: &str, url: &str) -> Result<Vec<u8>, String> {
    host::log(
        host::LogLevel::Debug,
        &format!("Drive API raw: {} {}", method, url),
    );

    let response = host::http_request(method, url, "{}", None, None)?;

    if response.status < 200 || response.status >= 300 {
        let body_text = String::from_utf8_lossy(&response.body);
        return Err(format!(
            "Drive API returned status {}: {}",
            response.status, body_text
        ));
    }

    Ok(response.body)
}

/// Parse a file resource from the API response.
fn parse_file(v: &serde_json::Value) -> DriveFile {
    let mime_type = v["mimeType"].as_str().unwrap_or("").to_string();
    DriveFile {
        id: v["id"].as_str().unwrap_or("").to_string(),
        name: v["name"].as_str().unwrap_or("").to_string(),
        is_folder: mime_type == "application/vnd.google-apps.folder",
        mime_type,
        description: v["description"].as_str().map(|s| s.to_string()),
        size: v["size"].as_str().map(|s| s.to_string()),
        created_time: v["createdTime"].as_str().map(|s| s.to_string()),
        modified_time: v["modifiedTime"].as_str().map(|s| s.to_string()),
        web_view_link: v["webViewLink"].as_str().map(|s| s.to_string()),
        parents: v["parents"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| p.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
        shared: v["shared"].as_bool().unwrap_or(false),
        starred: v["starred"].as_bool().unwrap_or(false),
        trashed: v["trashed"].as_bool().unwrap_or(false),
        owned_by_me: v["ownedByMe"].as_bool().unwrap_or(false),
        drive_id: v["driveId"].as_str().map(|s| s.to_string()),
        owners: v["owners"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|o| Owner {
                        email: o["emailAddress"].as_str().unwrap_or("").to_string(),
                        display_name: o["displayName"].as_str().map(|s| s.to_string()),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// List/search files.
pub fn list_files(
    query: Option<&str>,
    page_size: u32,
    order_by: Option<&str>,
    corpora: &str,
    drive_id: Option<&str>,
    page_token: Option<&str>,
) -> Result<ListFilesResult, String> {
    let mut params = vec![
        format!("pageSize={}", page_size),
        format!("fields=nextPageToken,files({})", FILE_FIELDS),
        format!("corpora={}", corpora),
        "supportsAllDrives=true".to_string(),
        "includeItemsFromAllDrives=true".to_string(),
    ];

    if let Some(q) = query {
        params.push(format!("q={}", url_encode(q)));
    }
    if let Some(ob) = order_by {
        params.push(format!("orderBy={}", url_encode(ob)));
    }
    if let Some(did) = drive_id {
        params.push(format!("driveId={}", url_encode(did)));
    }
    if let Some(pt) = page_token {
        params.push(format!("pageToken={}", url_encode(pt)));
    }

    let path = format!("files?{}", params.join("&"));
    let response = api_call("GET", &path, None)?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

    let files = parsed["files"]
        .as_array()
        .map(|arr| arr.iter().map(parse_file).collect())
        .unwrap_or_default();

    Ok(ListFilesResult {
        files,
        next_page_token: parsed["nextPageToken"].as_str().map(|s| s.to_string()),
    })
}

/// Get file metadata.
pub fn get_file(file_id: &str) -> Result<FileResult, String> {
    let path = format!(
        "files/{}?fields={}&supportsAllDrives=true",
        url_encode(file_id),
        FILE_FIELDS
    );
    let response = api_call("GET", &path, None)?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

    Ok(FileResult {
        file: parse_file(&parsed),
    })
}

/// Download file content as text.
pub fn download_file(
    file_id: &str,
    export_mime_type: Option<&str>,
) -> Result<DownloadResult, String> {
    // First get metadata to know the file type and name
    let meta = get_file(file_id)?;
    let mime = &meta.file.mime_type;

    let bytes = if mime.starts_with("application/vnd.google-apps.") {
        // Google Workspace file, must export
        let export_type = export_mime_type.unwrap_or(match mime.as_str() {
            "application/vnd.google-apps.document" => "text/plain",
            "application/vnd.google-apps.spreadsheet" => "text/csv",
            "application/vnd.google-apps.presentation" => "text/plain",
            "application/vnd.google-apps.drawing" => "image/svg+xml",
            _ => "text/plain",
        });
        let url = format!(
            "{}/files/{}/export?mimeType={}",
            DRIVE_API_BASE,
            url_encode(file_id),
            url_encode(export_type)
        );
        api_call_raw("GET", &url)?
    } else {
        // Regular file, download directly
        let url = format!("{}/files/{}?alt=media", DRIVE_API_BASE, url_encode(file_id));
        api_call_raw("GET", &url)?
    };

    let content = String::from_utf8(bytes).map_err(|_| {
        "File content is binary, cannot display as text. Use get_file for metadata only."
            .to_string()
    })?;

    Ok(DownloadResult {
        file_id: file_id.to_string(),
        name: meta.file.name,
        mime_type: meta.file.mime_type,
        content,
    })
}

/// Upload a text file using multipart upload.
pub fn upload_file(
    name: &str,
    content: &str,
    mime_type: &str,
    parent_id: Option<&str>,
    description: Option<&str>,
) -> Result<FileResult, String> {
    let boundary = "ironclaw_upload_boundary_42";

    let mut metadata = serde_json::json!({
        "name": name,
        "mimeType": mime_type,
    });
    if let Some(pid) = parent_id {
        metadata["parents"] = serde_json::json!([pid]);
    }
    if let Some(desc) = description {
        metadata["description"] = serde_json::Value::String(desc.to_string());
    }

    let metadata_str = serde_json::to_string(&metadata).map_err(|e| e.to_string())?;

    // Build multipart body
    let mut body = String::new();
    body.push_str(&format!("--{}\r\n", boundary));
    body.push_str("Content-Type: application/json; charset=UTF-8\r\n\r\n");
    body.push_str(&metadata_str);
    body.push_str(&format!("\r\n--{}\r\n", boundary));
    body.push_str(&format!("Content-Type: {}\r\n\r\n", mime_type));
    body.push_str(content);
    body.push_str(&format!("\r\n--{}--", boundary));

    let url = format!(
        "{}/files?uploadType=multipart&fields={}&supportsAllDrives=true",
        UPLOAD_API_BASE, FILE_FIELDS
    );
    let headers = format!(
        r#"{{"Content-Type": "multipart/related; boundary={}"}}"#,
        boundary
    );

    host::log(
        host::LogLevel::Debug,
        "Drive API: POST upload/files (multipart)",
    );

    let response = host::http_request("POST", &url, &headers, Some(body.as_bytes()), None)?;

    if response.status < 200 || response.status >= 300 {
        let body_text = String::from_utf8_lossy(&response.body);
        return Err(format!(
            "Upload failed with status {}: {}",
            response.status, body_text
        ));
    }

    let parsed: serde_json::Value = serde_json::from_str(
        &String::from_utf8(response.body).map_err(|e| format!("Invalid UTF-8: {}", e))?,
    )
    .map_err(|e| format!("Failed to parse response: {}", e))?;

    Ok(FileResult {
        file: parse_file(&parsed),
    })
}

/// Update file metadata.
pub fn update_file(
    file_id: &str,
    name: Option<&str>,
    description: Option<&str>,
    move_to_parent: Option<&str>,
    starred: Option<bool>,
) -> Result<FileResult, String> {
    let mut patch = serde_json::json!({});

    if let Some(n) = name {
        patch["name"] = serde_json::Value::String(n.to_string());
    }
    if let Some(d) = description {
        patch["description"] = serde_json::Value::String(d.to_string());
    }
    if let Some(s) = starred {
        patch["starred"] = serde_json::Value::Bool(s);
    }

    let mut params = vec![
        format!("fields={}", FILE_FIELDS),
        "supportsAllDrives=true".to_string(),
    ];

    if let Some(new_parent) = move_to_parent {
        // To move, we need to know current parents first
        let current = get_file(file_id)?;
        let remove_parents = current
            .file
            .parents
            .iter()
            .map(|p| p.as_str())
            .collect::<Vec<_>>()
            .join(",");
        params.push(format!("addParents={}", url_encode(new_parent)));
        if !remove_parents.is_empty() {
            params.push(format!("removeParents={}", url_encode(&remove_parents)));
        }
    }

    let body = serde_json::to_string(&patch).map_err(|e| e.to_string())?;
    let path = format!("files/{}?{}", url_encode(file_id), params.join("&"));

    let response = api_call("PATCH", &path, Some(&body))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

    Ok(FileResult {
        file: parse_file(&parsed),
    })
}

/// Create a folder.
pub fn create_folder(
    name: &str,
    parent_id: Option<&str>,
    description: Option<&str>,
) -> Result<FileResult, String> {
    let mut metadata = serde_json::json!({
        "name": name,
        "mimeType": "application/vnd.google-apps.folder",
    });
    if let Some(pid) = parent_id {
        metadata["parents"] = serde_json::json!([pid]);
    }
    if let Some(desc) = description {
        metadata["description"] = serde_json::Value::String(desc.to_string());
    }

    let body = serde_json::to_string(&metadata).map_err(|e| e.to_string())?;
    let path = format!("files?fields={}&supportsAllDrives=true", FILE_FIELDS);

    let response = api_call("POST", &path, Some(&body))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

    Ok(FileResult {
        file: parse_file(&parsed),
    })
}

/// Delete a file permanently.
pub fn delete_file(file_id: &str) -> Result<DeleteResult, String> {
    let path = format!("files/{}?supportsAllDrives=true", url_encode(file_id));
    api_call("DELETE", &path, None)?;

    Ok(DeleteResult {
        file_id: file_id.to_string(),
        deleted: true,
    })
}

/// Move a file to trash.
pub fn trash_file(file_id: &str) -> Result<DeleteResult, String> {
    let body = r#"{"trashed": true}"#;
    let path = format!(
        "files/{}?fields={}&supportsAllDrives=true",
        url_encode(file_id),
        FILE_FIELDS
    );

    api_call("PATCH", &path, Some(body))?;

    Ok(DeleteResult {
        file_id: file_id.to_string(),
        deleted: true,
    })
}

/// Share a file with someone.
pub fn share_file(
    file_id: &str,
    email: &str,
    role: &str,
    message: Option<&str>,
) -> Result<ShareResult, String> {
    let permission = serde_json::json!({
        "type": "user",
        "role": role,
        "emailAddress": email,
    });

    let body = serde_json::to_string(&permission).map_err(|e| e.to_string())?;

    let mut path = format!(
        "files/{}/permissions?supportsAllDrives=true",
        url_encode(file_id)
    );
    if let Some(msg) = message {
        path.push_str(&format!("&emailMessage={}", url_encode(msg)));
    }

    let response = api_call("POST", &path, Some(&body))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

    Ok(ShareResult {
        permission_id: parsed["id"].as_str().unwrap_or("").to_string(),
        role: parsed["role"].as_str().unwrap_or(role).to_string(),
        email: email.to_string(),
    })
}

/// List permissions on a file.
pub fn list_permissions(file_id: &str) -> Result<ListPermissionsResult, String> {
    let path = format!(
        "files/{}/permissions?fields=permissions(id,role,type,emailAddress,displayName)&supportsAllDrives=true",
        url_encode(file_id)
    );

    let response = api_call("GET", &path, None)?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

    let permissions = parsed["permissions"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|p| Permission {
                    id: p["id"].as_str().unwrap_or("").to_string(),
                    role: p["role"].as_str().unwrap_or("").to_string(),
                    permission_type: p["type"].as_str().unwrap_or("").to_string(),
                    email_address: p["emailAddress"].as_str().map(|s| s.to_string()),
                    display_name: p["displayName"].as_str().map(|s| s.to_string()),
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(ListPermissionsResult { permissions })
}

/// Remove a sharing permission.
pub fn remove_permission(file_id: &str, permission_id: &str) -> Result<DeleteResult, String> {
    let path = format!(
        "files/{}/permissions/{}?supportsAllDrives=true",
        url_encode(file_id),
        url_encode(permission_id)
    );

    api_call("DELETE", &path, None)?;

    Ok(DeleteResult {
        file_id: file_id.to_string(),
        deleted: true,
    })
}

/// List shared drives.
pub fn list_shared_drives(page_size: u32) -> Result<ListSharedDrivesResult, String> {
    let path = format!("drives?pageSize={}", page_size);
    let response = api_call("GET", &path, None)?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

    let drives = parsed["drives"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|d| SharedDrive {
                    id: d["id"].as_str().unwrap_or("").to_string(),
                    name: d["name"].as_str().unwrap_or("").to_string(),
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(ListSharedDrivesResult { drives })
}

/// Minimal percent-encoding for URL path segments and query values.
fn url_encode(s: &str) -> String {
    let mut encoded = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(b as char);
            }
            _ => {
                encoded.push('%');
                encoded.push(char::from(HEX[(b >> 4) as usize]));
                encoded.push(char::from(HEX[(b & 0x0F) as usize]));
            }
        }
    }
    encoded
}

const HEX: [u8; 16] = *b"0123456789ABCDEF";
