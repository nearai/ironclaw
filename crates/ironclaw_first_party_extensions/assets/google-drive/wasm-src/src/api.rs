//! Google Drive API v3 implementation.
//!
//! All API calls go through the host's HTTP capability, which handles
//! credential injection and rate limiting. The WASM tool never sees
//! the actual OAuth token.

use base64::Engine as _;

use crate::near::agent::host;
use crate::types::*;

const DRIVE_API_BASE: &str = "https://www.googleapis.com/drive/v3";
const UPLOAD_API_BASE: &str = "https://www.googleapis.com/upload/drive/v3";
const MAX_DOWNLOAD_TEXT_BYTES: usize = 1_000_000;
const GOOGLE_API_AUTH_REQUIRED_ERROR: &str = "google_api_error_status_401";

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
        return Err(api_status_error("Drive", response.status, &response.body));
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
        return Err(api_status_error("Drive", response.status, &response.body));
    }

    Ok(response.body)
}

fn api_status_error(service: &str, status: u16, body: &[u8]) -> String {
    if status == 401 {
        return serde_json::json!({
            "code": GOOGLE_API_AUTH_REQUIRED_ERROR,
            "kind": "auth_required",
        })
        .to_string();
    }
    let body_text = String::from_utf8_lossy(body);
    format!("{service} API returned status {status}: {body_text}")
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
    let meta = get_file(file_id)?;
    let is_google_apps = meta
        .file
        .mime_type
        .starts_with("application/vnd.google-apps.");

    // Regular files declare their size up front. If it exceeds the inline cap,
    // return an honest, model-visible message (a successful result whose content
    // explains the limit) rather than failing the capability opaquely — and skip
    // the wasted download.
    if !is_google_apps {
        if let Some(message) = declared_oversize_message(meta.file.size.as_deref()) {
            return Ok(oversize_result(meta, message));
        }
    }

    let bytes = if is_google_apps {
        // Google Workspace file, must export
        let export_type = export_mime_type.unwrap_or(match meta.file.mime_type.as_str() {
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

    // Exports have no declared size, and a declared size can be wrong, so guard
    // again on the actual downloaded byte count.
    if let Some(message) = oversize_message(bytes.len() as u64) {
        return Ok(oversize_result(meta, message));
    }

    // Text files (and exported Google Workspace files) decode as UTF-8 and are
    // returned inline. Binary files (PDF, PPTX, DOCX, ...) cannot be shown as
    // text here — return their raw bytes base64-encoded so the host runtime can
    // run them through the document text extractor and swap in the extracted
    // text before the result reaches the model. The guest never decides what is
    // extractable; the host does.
    match String::from_utf8(bytes) {
        Ok(content) => Ok(DownloadResult {
            file_id: file_id.to_string(),
            name: meta.file.name,
            mime_type: meta.file.mime_type,
            content: Some(content),
            content_base64: None,
        }),
        Err(err) => {
            let encoded = base64::engine::general_purpose::STANDARD.encode(err.into_bytes());
            Ok(DownloadResult {
                file_id: file_id.to_string(),
                name: meta.file.name,
                mime_type: meta.file.mime_type,
                content: None,
                content_base64: Some(encoded),
            })
        }
    }
}

/// If a declared metadata size exceeds the inline cap, return a model-visible
/// "too large" message. `None` when the size is absent, unparseable, or within
/// the cap.
fn declared_oversize_message(size: Option<&str>) -> Option<String> {
    // Parse as u64: `usize` is 32-bit on wasm32, so a >4 GB declared size would
    // overflow and skip this guard. silent-ok: a malformed/oversized declared
    // size just falls through to the post-download byte-count guard.
    let parsed = size?.parse::<u64>().ok()?;
    oversize_message(parsed)
}

/// If `size` exceeds the inline cap, return a model-visible "too large"
/// message; otherwise `None`.
fn oversize_message(size: u64) -> Option<String> {
    if size <= MAX_DOWNLOAD_TEXT_BYTES as u64 {
        return None;
    }
    let mb = size as f64 / (1024.0 * 1024.0);
    let limit_mb = MAX_DOWNLOAD_TEXT_BYTES as f64 / (1024.0 * 1024.0);
    Some(format!(
        "[File too large to read inline: {mb:.1} MB exceeds the {limit_mb:.0} MB limit for \
         reading documents directly. Ask the user to share a smaller file or paste the \
         relevant excerpt.]"
    ))
}

/// A successful download result that carries an explanatory message in place of
/// content (used for the too-large case). The model sees the message verbatim.
fn oversize_result(meta: FileResult, message: String) -> DownloadResult {
    DownloadResult {
        file_id: meta.file.id,
        name: meta.file.name,
        mime_type: meta.file.mime_type,
        content: Some(message),
        content_base64: None,
    }
}

/// Upload a text file using multipart upload.
pub fn upload_file(
    name: &str,
    content: &str,
    mime_type: &str,
    parent_id: Option<&str>,
    description: Option<&str>,
) -> Result<FileResult, String> {
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
    let boundary = multipart_boundary(&metadata_str, content);

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
        return Err(api_status_error("Drive", response.status, &response.body));
    }

    let parsed: serde_json::Value = serde_json::from_str(
        &String::from_utf8(response.body).map_err(|e| format!("Invalid UTF-8: {}", e))?,
    )
    .map_err(|e| format!("Failed to parse response: {}", e))?;

    Ok(FileResult {
        file: parse_file(&parsed),
    })
}

fn multipart_boundary(metadata: &str, content: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for b in metadata.bytes().chain(content.bytes()) {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }

    let mut attempt = 0_u64;
    loop {
        let boundary = format!("ironclaw_upload_boundary_{hash:016x}_{attempt:016x}");
        if !metadata.contains(&boundary) && !content.contains(&boundary) {
            return boundary;
        }
        attempt = attempt.wrapping_add(1);
    }
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

fn url_encode(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multipart_boundary_does_not_appear_in_metadata_or_content() {
        let metadata = r#"{"name":"ironclaw_upload_boundary_marker"}"#;
        let content = "body with ironclaw_upload_boundary_marker";

        let boundary = multipart_boundary(metadata, content);

        assert!(!metadata.contains(&boundary));
        assert!(!content.contains(&boundary));
        assert!(boundary.starts_with("ironclaw_upload_boundary_"));
    }

    #[test]
    fn within_cap_has_no_oversize_message() {
        assert!(oversize_message(0).is_none());
        assert!(oversize_message(MAX_DOWNLOAD_TEXT_BYTES as u64).is_none());
    }

    #[test]
    fn over_cap_yields_honest_model_visible_message() {
        // A 10 MB file should produce a clear, actionable message — not the
        // opaque `operation_failed` the host derives from a guest `Err`.
        let message = oversize_message(10 * 1024 * 1024).expect("over-cap returns a message");
        assert!(
            message.contains("too large to read inline"),
            "expected honest phrasing, got: {message}"
        );
        assert!(
            message.contains("10.0 MB"),
            "should state the actual size: {message}"
        );
    }

    #[test]
    fn declared_oversize_message_parses_and_gates_on_size() {
        // Absent or unparseable declared size -> no message (fall through to the
        // post-download byte-count guard); within cap -> none; over cap -> some.
        assert!(declared_oversize_message(None).is_none());
        assert!(declared_oversize_message(Some("not-a-number")).is_none());
        assert!(declared_oversize_message(Some("500000")).is_none());
        assert!(declared_oversize_message(Some("10485760")).is_some());
        // Multi-GB sizes must parse (u64), not overflow `usize` on wasm32 and
        // silently skip the guard.
        assert!(declared_oversize_message(Some("5000000000")).is_some());
    }
}
