//! Slack external file-upload sequence for transient workspace attachments.

use ironclaw_product_adapters::{
    DeclaredEgressHost, EgressCredentialHandle, EgressHeader, EgressMethod, EgressPath,
    EgressRequest, ProductAdapterError, ProductOutboundAttachment, ProtocolHttpEgress,
    RedactedString,
};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::payload::{SLACK_API_HOST, SLACK_FILES_HOST};
use crate::render::SlackReplyTarget;

pub(crate) async fn upload_workspace_file(
    egress: &dyn ProtocolHttpEgress,
    credential_handle: EgressCredentialHandle,
    target: &SlackReplyTarget,
    attachment: &ProductOutboundAttachment,
) -> Result<(), ProductAdapterError> {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("filename", attachment.filename())
        .append_pair("length", &attachment.bytes().len().to_string())
        .finish();
    let response = egress
        .send(slack_request(
            SLACK_API_HOST,
            "GET",
            format!("/api/files.getUploadURLExternal?{query}"),
            None,
            Some(credential_handle.clone()),
            Some(64 * 1024),
        )?)
        .await?;
    let upload: UploadUrlResponse = parse_slack_json(response.status(), response.body())?;
    if !upload.ok {
        return Err(permanent("Slack rejected the file upload URL request"));
    }
    let upload_url = upload
        .upload_url
        .ok_or_else(|| permanent("Slack omitted the file upload URL"))?;
    let file_id = upload
        .file_id
        .ok_or_else(|| permanent("Slack omitted the uploaded file id"))?;
    let (host, path) = confined_upload_url(&upload_url)?;
    let response = egress
        .send(slack_request(
            &host,
            "POST",
            path,
            Some(("application/octet-stream", attachment.bytes().to_vec())),
            None,
            Some(64 * 1024),
        )?)
        .await?;
    if !(200..300).contains(&response.status()) {
        return Err(http_failure(
            response.status(),
            "Slack file-byte upload failed",
        ));
    }

    let body = serde_json::to_vec(&CompleteUploadRequest {
        files: vec![CompletedFile {
            id: file_id,
            title: attachment.filename().to_string(),
        }],
        channel_id: target.channel.clone(),
        thread_ts: target.thread_ts.clone(),
    })
    .map_err(|_| permanent("Slack file completion request could not be encoded"))?;
    let response = egress
        .send(slack_request(
            SLACK_API_HOST,
            "POST",
            "/api/files.completeUploadExternal".to_string(),
            Some(("application/json", body)),
            Some(credential_handle),
            Some(64 * 1024),
        )?)
        .await?;
    let completed: SlackOkResponse = parse_slack_json(response.status(), response.body())?;
    if !completed.ok {
        return Err(permanent("Slack rejected file upload completion"));
    }
    Ok(())
}

fn slack_request(
    host: &str,
    method: &str,
    path: String,
    body: Option<(&str, Vec<u8>)>,
    credential_handle: Option<EgressCredentialHandle>,
    response_limit: Option<u64>,
) -> Result<EgressRequest, ProductAdapterError> {
    let mut request = EgressRequest::new(
        DeclaredEgressHost::new(host)?,
        EgressMethod::new(method)?,
        EgressPath::new(path)?,
    )
    .with_credential_handle(credential_handle);
    if let Some((content_type, body)) = body {
        request = request
            .with_header(EgressHeader::new("content-type", content_type)?)
            .with_body(body);
    }
    if let Some(limit) = response_limit {
        request = request.with_response_body_limit(limit);
    }
    Ok(request)
}

fn confined_upload_url(raw: &str) -> Result<(String, String), ProductAdapterError> {
    let parsed = Url::parse(raw).map_err(|_| permanent("Slack returned an invalid upload URL"))?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some(SLACK_FILES_HOST)
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.port().is_some()
        || parsed.fragment().is_some()
    {
        return Err(permanent("Slack upload URL escaped the allowed file host"));
    }
    let mut path = parsed.path().to_string();
    if let Some(query) = parsed.query() {
        path.push('?');
        path.push_str(query);
    }
    Ok((SLACK_FILES_HOST.to_string(), path))
}

fn parse_slack_json<T: for<'de> Deserialize<'de>>(
    status: u16,
    body: &[u8],
) -> Result<T, ProductAdapterError> {
    if !(200..300).contains(&status) {
        return Err(http_failure(status, "Slack file API request failed"));
    }
    if body.len() > 64 * 1024 {
        return Err(permanent("Slack file API response exceeded its size limit"));
    }
    serde_json::from_slice(body)
        .map_err(|_| permanent("Slack file API returned an invalid response"))
}

fn http_failure(status: u16, reason: &'static str) -> ProductAdapterError {
    if status >= 500 || status == 429 || status == 408 {
        ProductAdapterError::EgressTransient {
            reason: RedactedString::new(reason),
        }
    } else {
        permanent(reason)
    }
}

fn permanent(reason: &'static str) -> ProductAdapterError {
    ProductAdapterError::EgressDenied {
        reason: RedactedString::new(reason),
    }
}

#[derive(Deserialize)]
struct UploadUrlResponse {
    ok: bool,
    upload_url: Option<String>,
    file_id: Option<String>,
}

#[derive(Deserialize)]
struct SlackOkResponse {
    ok: bool,
}

#[derive(Serialize)]
struct CompleteUploadRequest {
    files: Vec<CompletedFile>,
    channel_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_ts: Option<String>,
}

#[derive(Serialize)]
struct CompletedFile {
    id: String,
    title: String,
}
