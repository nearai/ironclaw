use serde_json::Value;

use super::{
    CodingCapabilityError, input_error, inputs::required_str, operation_error_with_summary,
    text::ReplaceContentError,
};

const MAX_PATCH_EDITS: usize = 256;

#[derive(Debug, Clone)]
pub(super) struct PatchEdit {
    pub(super) old_string: String,
    pub(super) new_string: String,
}

#[derive(Debug, Clone)]
pub(super) struct ApplyPatchInput {
    pub(super) edits: Vec<PatchEdit>,
    pub(super) replace_all: bool,
}

pub(super) fn parse_apply_patch_input(
    input: &Value,
) -> Result<ApplyPatchInput, CodingCapabilityError> {
    let replace_all = match input.get("replace_all") {
        Some(Value::Bool(value)) => *value,
        Some(_) => return Err(input_error()),
        None => false,
    };

    let edits = if let Some(edits_value) = optional_edits_value(input) {
        if optional_sentinel_field(input, "old_string").is_some()
            || optional_sentinel_field(input, "new_string").is_some()
        {
            return Err(input_error());
        }
        parse_patch_edits(edits_value)?
    } else {
        let old_raw = required_str(input, "old_string")?;
        let new_raw = required_str(input, "new_string")?;
        if old_raw == "null" || new_raw == "null" {
            return Err(input_error());
        }
        let old_string = normalize_patch_text(old_raw);
        let new_string = normalize_patch_text(new_raw);
        vec![validated_patch_edit(old_string, new_string)?]
    };

    if edits.is_empty() || edits.len() > MAX_PATCH_EDITS || replace_all && edits.len() != 1 {
        return Err(input_error());
    }

    Ok(ApplyPatchInput { edits, replace_all })
}

pub(super) fn replacement_error(
    error: ReplaceContentError,
    safe_path: String,
    edit_count: usize,
) -> CodingCapabilityError {
    match error {
        ReplaceContentError::EmptyOld
        | ReplaceContentError::InvalidEditCount
        | ReplaceContentError::NoChange => input_error(),
        ReplaceContentError::NotFound { edit_index } => operation_error_with_summary(format!(
            "apply_patch failed for {safe_path}: {} matched 0 times",
            edit_label(edit_index, edit_count)
        )),
        ReplaceContentError::Duplicate {
            edit_index,
            occurrences,
        } => operation_error_with_summary(format!(
            "apply_patch failed for {safe_path}: {} matched {occurrences} times; set replace_all=true or provide a unique old_string",
            edit_label(edit_index, edit_count)
        )),
        ReplaceContentError::Overlap {
            previous_edit_index,
            current_edit_index,
        } => operation_error_with_summary(format!(
            "apply_patch failed for {safe_path}: edits[{previous_edit_index}] and edits[{current_edit_index}] overlap; merge them into one edit"
        )),
    }
}

fn parse_patch_edits(edits_value: &Value) -> Result<Vec<PatchEdit>, CodingCapabilityError> {
    let parsed;
    let edits_value = if let Some(edits_string) = edits_value.as_str() {
        parsed = serde_json::from_str::<Value>(edits_string).map_err(|error| {
            tracing::debug!(?error, "apply_patch edits string is not valid JSON");
            input_error()
        })?;
        &parsed
    } else {
        edits_value
    };

    let edits = edits_value.as_array().ok_or_else(input_error)?;
    if edits.is_empty() {
        return Err(input_error());
    }

    edits
        .iter()
        .map(parse_patch_edit)
        .collect::<Result<Vec<_>, _>>()
}

fn optional_edits_value(input: &Value) -> Option<&Value> {
    optional_sentinel_field(input, "edits")
}

fn optional_sentinel_field<'a>(input: &'a Value, field: &str) -> Option<&'a Value> {
    match input.get(field) {
        Some(Value::Null) => None,
        Some(Value::String(value)) if value == "null" => None,
        value => value,
    }
}

fn parse_patch_edit(edit: &Value) -> Result<PatchEdit, CodingCapabilityError> {
    let snake = patch_edit_from_fields(edit, "old_string", "new_string")?;
    let camel = patch_edit_from_fields(edit, "oldText", "newText")?;
    match (snake, camel) {
        (Some(_), Some(_)) => Err(input_error()),
        (Some(edit), None) | (None, Some(edit)) => validated_patch_edit(edit.0, edit.1),
        (None, None) => Err(input_error()),
    }
}

fn patch_edit_from_fields(
    edit: &Value,
    old_field: &str,
    new_field: &str,
) -> Result<Option<(String, String)>, CodingCapabilityError> {
    let old = edit.get(old_field);
    let new = edit.get(new_field);
    match (old, new) {
        (None, None) => Ok(None),
        (Some(old), Some(new)) => {
            let old = old.as_str().ok_or_else(input_error)?;
            let new = new.as_str().ok_or_else(input_error)?;
            Ok(Some((normalize_patch_text(old), normalize_patch_text(new))))
        }
        _ => Err(input_error()),
    }
}

fn validated_patch_edit(
    old_string: String,
    new_string: String,
) -> Result<PatchEdit, CodingCapabilityError> {
    if old_string.is_empty() || old_string == new_string {
        return Err(input_error());
    }
    Ok(PatchEdit {
        old_string,
        new_string,
    })
}

fn normalize_patch_text(value: &str) -> String {
    value.replace("\r\n", "\n").replace('\r', "\n")
}

fn edit_label(edit_index: usize, edit_count: usize) -> String {
    if edit_count == 1 {
        "old_string".to_string()
    } else {
        format!("edits[{edit_index}].old_string")
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parse_apply_patch_input_treats_null_edits_as_absent() {
        for edits in [Value::Null, Value::String("null".to_string())] {
            let input = json!({
                "path": "/workspace/main.txt",
                "old_string": "old",
                "new_string": "new",
                "edits": edits
            });

            let parsed = parse_apply_patch_input(&input).expect("single edit");

            assert_eq!(parsed.edits.len(), 1);
            assert_eq!(parsed.edits[0].old_string, "old");
            assert_eq!(parsed.edits[0].new_string, "new");
        }
    }

    #[test]
    fn parse_apply_patch_input_rejects_non_boolean_replace_all() {
        let input = json!({
            "path": "/workspace/main.txt",
            "old_string": "old",
            "new_string": "new",
            "replace_all": "true"
        });

        assert!(parse_apply_patch_input(&input).is_err());
    }

    #[test]
    fn parse_apply_patch_input_rejects_active_null_string_placeholders() {
        for (old_string, new_string) in [("null", "new"), ("old", "null")] {
            let input = json!({
                "path": "/workspace/main.txt",
                "old_string": old_string,
                "new_string": new_string
            });

            assert!(parse_apply_patch_input(&input).is_err());
        }
    }

    #[test]
    fn parse_apply_patch_input_rejects_too_many_edits() {
        let edits = (0..=MAX_PATCH_EDITS)
            .map(|index| json!({"old_string": format!("old {index}"), "new_string": "new"}))
            .collect::<Vec<_>>();
        let input = json!({
            "path": "/workspace/main.txt",
            "edits": edits
        });

        assert!(parse_apply_patch_input(&input).is_err());
    }

    #[test]
    fn parse_apply_patch_input_treats_top_level_null_strings_as_absent_for_edits() {
        let input = json!({
            "path": "/workspace/main.txt",
            "old_string": "null",
            "new_string": Value::Null,
            "edits": [
                {"old_string": "old", "new_string": "new"}
            ]
        });

        let parsed = parse_apply_patch_input(&input).expect("multi edit");

        assert_eq!(parsed.edits.len(), 1);
        assert_eq!(parsed.edits[0].old_string, "old");
        assert_eq!(parsed.edits[0].new_string, "new");
    }
}
