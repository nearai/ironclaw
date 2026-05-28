use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{EffectKind, PermissionMode, RuntimeDispatchErrorKind};
use serde_json::{Value, json};

use crate::{FirstPartyCapabilityError, FirstPartyCapabilityRequest, RuntimeProcessError};

use super::{first_party_capability_manifest, input_error, operation_error};

pub const SAVED_OUTPUT_READ_CAPABILITY_ID: &str = "builtin.saved_output_read";

pub(super) fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    first_party_capability_manifest(
        SAVED_OUTPUT_READ_CAPABILITY_ID,
        "Read a previously returned shell saved-output ref through scoped runtime checks",
        vec![EffectKind::ReadFilesystem],
        PermissionMode::Allow,
        None,
    )
}

pub(super) fn dispatch(
    request: &FirstPartyCapabilityRequest,
) -> Result<Value, FirstPartyCapabilityError> {
    let input = request.input.as_object().ok_or_else(input_error)?;
    let ref_id = input
        .get("ref")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(input_error)?;
    let offset = optional_usize(input.get("offset_bytes"))?.unwrap_or(0);
    let limit = optional_usize(input.get("limit_bytes"))?
        .unwrap_or(crate::process_output::SAVED_OUTPUT_READ_MAX_BYTES);
    let read = crate::process_output::read_saved_output_ref(&request.scope, ref_id, offset, limit)
        .map_err(saved_output_error)?;
    Ok(json!({
        "ref": ref_id,
        "content_base64": BASE64_STANDARD.encode(&read.content),
        "encoding": "base64",
        "offset_bytes": read.offset,
        "next_offset_bytes": read.next_offset,
        "total_bytes": read.total_bytes,
    }))
}

fn optional_usize(value: Option<&Value>) -> Result<Option<usize>, FirstPartyCapabilityError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(value) = value.as_u64() else {
        return Err(input_error());
    };
    usize::try_from(value).map(Some).map_err(|_| input_error())
}

fn saved_output_error(error: RuntimeProcessError) -> FirstPartyCapabilityError {
    match error {
        RuntimeProcessError::ExecutionFailed(reason)
            if reason.contains("invalid saved output ref") =>
        {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::InputEncode)
        }
        RuntimeProcessError::ExecutionFailed(_) => operation_error(),
        RuntimeProcessError::Timeout(_) => {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Resource)
        }
    }
}
