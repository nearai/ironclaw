use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityFailureDetail, CapabilityInputIssue,
    CapabilityInputIssueCode,
};

pub(super) const MAX_PROVIDER_NORMALIZATION_DEPTH: usize = 32;
const MAX_PROVIDER_SCHEMA_ISSUES: usize = 3;

pub(super) fn prepare_provider_arguments(
    arguments: &serde_json::Value,
    schema: &serde_json::Value,
    label: &'static str,
) -> Result<serde_json::Value, AgentLoopHostError> {
    prepare_provider_arguments_with_detail(arguments, schema, label).map_err(|error| error.error)
}

pub(super) fn prepare_provider_arguments_with_detail(
    arguments: &serde_json::Value,
    schema: &serde_json::Value,
    label: &'static str,
) -> Result<serde_json::Value, ProviderArgumentError> {
    let normalized = normalize_provider_arguments(arguments, schema, label)?;
    validate_provider_arguments_schema(&normalized, schema, label)?;
    Ok(normalized)
}

pub(super) struct ProviderArgumentError {
    pub(super) error: AgentLoopHostError,
    pub(super) detail: Option<CapabilityFailureDetail>,
}

impl From<AgentLoopHostError> for ProviderArgumentError {
    fn from(error: AgentLoopHostError) -> Self {
        Self {
            error,
            detail: None,
        }
    }
}

pub(super) fn normalize_provider_arguments(
    arguments: &serde_json::Value,
    schema: &serde_json::Value,
    label: &'static str,
) -> Result<serde_json::Value, AgentLoopHostError> {
    normalize_provider_value(arguments, schema, label, 0)
}

fn normalize_provider_value(
    value: &serde_json::Value,
    schema: &serde_json::Value,
    label: &'static str,
    depth: usize,
) -> Result<serde_json::Value, AgentLoopHostError> {
    if depth > MAX_PROVIDER_NORMALIZATION_DEPTH {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            format!("{label} exceeded maximum schema normalization depth"),
        ));
    }

    if schema_type_matches(schema, "object") {
        let object_value = coerce_json_string(value, label)?;
        let Some(object) = object_value.as_object() else {
            if is_json_container_string(value) {
                return Err(provider_coercion_error(label, "object"));
            }
            return Ok(object_value);
        };
        let Some(properties) = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
        else {
            return Ok(object_value);
        };
        let mut normalized = object.clone();
        for (property, property_schema) in properties {
            if let Some(property_value) = normalized.get(property).cloned() {
                normalized.insert(
                    property.clone(),
                    normalize_provider_value(&property_value, property_schema, label, depth + 1)?,
                );
            }
        }
        return Ok(serde_json::Value::Object(normalized));
    }

    if schema_type_matches(schema, "array") {
        let array_value = coerce_json_string(value, label)?;
        let Some(array) = array_value.as_array() else {
            if is_json_container_string(value) {
                return Err(provider_coercion_error(label, "array"));
            }
            return Ok(array_value);
        };
        let Some(items) = schema.get("items") else {
            return Ok(array_value);
        };
        return array
            .iter()
            .map(|item| normalize_provider_value(item, items, label, depth + 1))
            .collect::<Result<Vec<_>, _>>()
            .map(serde_json::Value::Array);
    }

    if schema_type_matches(schema, "integer") {
        return coerce_integer_string(value, label);
    }

    if schema_type_matches(schema, "number") {
        return coerce_number_string(value, label);
    }

    if schema_type_matches(schema, "boolean") {
        return coerce_boolean_string(value, label);
    }

    // `oneOf` / `anyOf`: pick the variant whose declared `type` matches the
    // value's shape, after attempting to coerce stringified containers. This
    // covers variant schemas such as `{ oneOf: [{type:object}, {type:array}] }`.
    // Object schemas that also carry `anyOf`/`allOf` are handled by the object
    // branch above so declared properties are still normalized before full
    // JSON Schema validation enforces the composed constraints.
    if let Some(variants) = schema_variants(schema) {
        return normalize_one_of_variants(value, variants, label, depth);
    }

    Ok(value.clone())
}

fn validate_provider_arguments_schema(
    arguments: &serde_json::Value,
    schema: &serde_json::Value,
    label: &'static str,
) -> Result<(), ProviderArgumentError> {
    if schema_contains_external_ref(schema, 0) {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::StaleSurface,
            format!("{label} schema contains an unresolved $ref"),
        )
        .into());
    }
    let validator = jsonschema::validator_for(schema).map_err(|error| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::StaleSurface,
            format!("{label} schema is invalid: {error}"),
        )
    })?;
    let mut errors = validator
        .iter_errors(arguments)
        .take(MAX_PROVIDER_SCHEMA_ISSUES);
    if let Some(error) = errors.next() {
        let instance_path = safe_schema_path_summary(error.instance_path().as_str());
        let schema_path = safe_schema_path_summary(error.schema_path().as_str());
        let mut issues = validation_error_input_issues(arguments, schema, &error);
        for error in errors {
            issues.extend(validation_error_input_issues(arguments, schema, &error));
            if issues.len() >= MAX_PROVIDER_SCHEMA_ISSUES {
                issues.truncate(MAX_PROVIDER_SCHEMA_ISSUES);
                break;
            }
        }
        return Err(ProviderArgumentError {
            error: AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                format!(
                    "{label} failed schema validation at instance path {instance_path} against schema path {schema_path}"
                ),
            ),
            detail: Some(CapabilityFailureDetail::InvalidInput { issues }),
        });
    }
    Ok(())
}

fn validation_error_input_issues(
    arguments: &serde_json::Value,
    schema: &serde_json::Value,
    error: &jsonschema::ValidationError<'_>,
) -> Vec<CapabilityInputIssue> {
    let schema_path = safe_schema_path_summary(error.schema_path().as_str());
    let (path, code, expected, received) = match error.kind().keyword() {
        "required" => return missing_required_issues(arguments, schema, error),
        "additionalProperties" => return unexpected_field_issues(arguments, schema, error),
        "type" => (
            safe_schema_path_summary(error.instance_path().as_str()),
            CapabilityInputIssueCode::TypeMismatch,
            expected_type_at_schema_path(schema, error.schema_path().as_str()),
            Some(json_value_kind(error.instance()).to_string()),
        ),
        _ => (
            safe_schema_path_summary(error.instance_path().as_str()),
            CapabilityInputIssueCode::InvalidValue,
            None,
            None,
        ),
    };

    vec![CapabilityInputIssue {
        path,
        code,
        expected,
        received,
        schema_path: Some(schema_path),
    }]
}

fn missing_required_issues(
    arguments: &serde_json::Value,
    schema: &serde_json::Value,
    error: &jsonschema::ValidationError<'_>,
) -> Vec<CapabilityInputIssue> {
    let instance_path = error.instance_path().as_str();
    let parent_path = safe_schema_path_summary(instance_path);
    let missing = missing_required_properties(arguments, schema, error);
    if missing.is_empty() {
        return vec![missing_required_issue(parent_path)];
    }
    missing
        .into_iter()
        .take(MAX_PROVIDER_SCHEMA_ISSUES)
        .map(|property| missing_required_issue(child_schema_path(&parent_path, &property)))
        .collect()
}

fn missing_required_issue(path: String) -> CapabilityInputIssue {
    CapabilityInputIssue {
        path,
        code: CapabilityInputIssueCode::MissingRequired,
        expected: Some("required field".to_string()),
        received: None,
        schema_path: Some("required".to_string()),
    }
}

fn unexpected_field_issues(
    arguments: &serde_json::Value,
    schema: &serde_json::Value,
    error: &jsonschema::ValidationError<'_>,
) -> Vec<CapabilityInputIssue> {
    let instance_path = error.instance_path().as_str();
    let parent_path = safe_schema_path_summary(instance_path);
    let schema_path = safe_schema_path_summary(error.schema_path().as_str());
    let Some(object) = value_at_json_pointer(arguments, instance_path).and_then(|v| v.as_object())
    else {
        return vec![invalid_value_issue(parent_path, schema_path)];
    };
    let Some(schema_object) =
        concrete_unexpected_field_parent_schema(schema, error.schema_path().as_str())
    else {
        return vec![invalid_value_issue(parent_path, schema_path)];
    };
    let properties = schema_object
        .get("properties")
        .and_then(serde_json::Value::as_object);
    let mut issues = object
        .keys()
        .filter(|property| !properties.is_some_and(|properties| properties.contains_key(*property)))
        .take(MAX_PROVIDER_SCHEMA_ISSUES)
        .map(|property| {
            unexpected_field_issue(
                child_schema_path(&parent_path, property),
                schema_path.clone(),
            )
        })
        .collect::<Vec<_>>();
    if issues.is_empty() {
        issues.push(invalid_value_issue(parent_path, schema_path));
    }
    issues
}

fn unexpected_field_issue(path: String, schema_path: String) -> CapabilityInputIssue {
    CapabilityInputIssue {
        path,
        code: CapabilityInputIssueCode::UnexpectedField,
        expected: Some("declared field".to_string()),
        received: Some("unexpected field".to_string()),
        schema_path: Some(schema_path),
    }
}

fn invalid_value_issue(path: String, schema_path: String) -> CapabilityInputIssue {
    CapabilityInputIssue {
        path,
        code: CapabilityInputIssueCode::InvalidValue,
        expected: None,
        received: None,
        schema_path: Some(schema_path),
    }
}

fn concrete_unexpected_field_parent_schema<'a>(
    schema: &'a serde_json::Value,
    schema_path: &str,
) -> Option<&'a serde_json::Map<String, serde_json::Value>> {
    let schema_object = schema_at_keyword_parent(schema, schema_path, "additionalProperties")?;
    if schema_object.get("additionalProperties") != Some(&serde_json::Value::Bool(false)) {
        return None;
    }
    if schema_object.contains_key("patternProperties")
        || schema_object.contains_key("propertyNames")
        || schema_object.contains_key("unevaluatedProperties")
        || schema_object.contains_key("allOf")
        || schema_object.contains_key("anyOf")
        || schema_object.contains_key("oneOf")
    {
        return None;
    }
    Some(schema_object)
}

fn missing_required_properties(
    arguments: &serde_json::Value,
    schema: &serde_json::Value,
    error: &jsonschema::ValidationError<'_>,
) -> Vec<String> {
    let Some(object) = value_at_json_pointer(arguments, error.instance_path().as_str())
        .and_then(|v| v.as_object())
    else {
        return Vec::new();
    };
    let Some(schema_object) =
        schema_at_keyword_parent(schema, error.schema_path().as_str(), "required")
    else {
        return Vec::new();
    };
    let Some(required) = schema_object
        .get("required")
        .and_then(serde_json::Value::as_array)
    else {
        return Vec::new();
    };
    required
        .iter()
        .filter_map(serde_json::Value::as_str)
        .filter(|property| !object.contains_key(*property))
        .take(MAX_PROVIDER_SCHEMA_ISSUES)
        .map(scrub_sensitive_schema_path_markers)
        .collect()
}

fn schema_at_keyword_parent<'a>(
    schema: &'a serde_json::Value,
    schema_path: &str,
    keyword: &str,
) -> Option<&'a serde_json::Map<String, serde_json::Value>> {
    let suffix = format!("/{keyword}");
    let parent_path = schema_path.strip_suffix(&suffix).unwrap_or(schema_path);
    value_at_json_pointer(schema, parent_path)?.as_object()
}

fn child_schema_path(parent_path: &str, child: &str) -> String {
    let child = safe_schema_path_summary(child);
    if parent_path == "root" {
        child
    } else {
        format!("{parent_path}.{child}")
    }
}

fn expected_type_at_schema_path(schema: &serde_json::Value, schema_path: &str) -> Option<String> {
    let type_path = if schema_path.ends_with("/type") {
        schema_path.to_string()
    } else {
        format!("{schema_path}/type")
    };
    let schema_type = value_at_json_pointer(schema, &type_path)?;
    if let Some(expected) = schema_type.as_str() {
        return Some(expected.to_string());
    }
    let expected = schema_type
        .as_array()?
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect::<Vec<_>>()
        .join(" or ");
    (!expected.is_empty()).then_some(expected)
}

fn value_at_json_pointer<'a>(
    value: &'a serde_json::Value,
    pointer: &str,
) -> Option<&'a serde_json::Value> {
    if pointer.is_empty() || pointer == "/" {
        Some(value)
    } else {
        value.pointer(pointer)
    }
}

fn json_value_kind(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

fn safe_schema_path_summary(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "/" {
        return "root".to_string();
    }
    let summary = trimmed
        .trim_start_matches('/')
        .replace(['/', '\\', '[', ']'], ".")
        .replace(['{', '}', '`', '<', '>'], "");
    scrub_sensitive_schema_path_markers(&summary)
}

fn scrub_sensitive_schema_path_markers(path: &str) -> String {
    let mut scrubbed = path.to_string();
    for marker in [
        "tool_input",
        "api_key",
        "apikey",
        "password",
        "passwd",
        "private_key",
        "private-key",
        "client_secret",
        "client-secret",
        "secret",
        "token",
        "bearer",
        "access_token",
        "access token",
    ] {
        scrubbed = replace_ascii_case_insensitive(&scrubbed, marker, "redacted");
    }
    scrubbed
}

fn replace_ascii_case_insensitive(input: &str, needle: &str, replacement: &str) -> String {
    let mut remaining = input;
    let mut replaced = String::with_capacity(input.len());
    while let Some(index) = remaining.to_ascii_lowercase().find(needle) {
        replaced.push_str(&remaining[..index]);
        replaced.push_str(replacement);
        remaining = &remaining[index + needle.len()..];
    }
    replaced.push_str(remaining);
    replaced
}

pub(super) fn schema_contains_external_ref(schema: &serde_json::Value, depth: usize) -> bool {
    if depth > MAX_PROVIDER_NORMALIZATION_DEPTH {
        return true;
    }
    match schema {
        serde_json::Value::Object(object) => {
            if object
                .get("$ref")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|reference| !reference.starts_with('#'))
            {
                return true;
            }
            object
                .values()
                .any(|value| schema_contains_external_ref(value, depth + 1))
        }
        serde_json::Value::Array(items) => items
            .iter()
            .any(|value| schema_contains_external_ref(value, depth + 1)),
        _ => false,
    }
}

fn schema_variants(schema: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    schema
        .get("oneOf")
        .or_else(|| schema.get("anyOf"))
        .and_then(serde_json::Value::as_array)
}

fn normalize_one_of_variants(
    value: &serde_json::Value,
    variants: &[serde_json::Value],
    label: &'static str,
    depth: usize,
) -> Result<serde_json::Value, AgentLoopHostError> {
    // Use `unwrap_or_else` rather than `?` so that an unparseable string
    // (e.g. a plain string that starts with `{` or `[` but is not valid
    // JSON) can still fall through to a `string` variant in the schema
    // instead of producing a false-positive `InvalidInvocation` error.
    let candidate = coerce_json_string(value, label).unwrap_or_else(|_| value.clone());
    let shape = value_shape(&candidate);
    for variant in variants {
        // In JSON Schema every `integer` is also a valid `number`, so allow
        // integer-shaped values to match `number` variants as well.
        if schema_type_matches(variant, shape)
            || (shape == "integer" && schema_type_matches(variant, "number"))
        {
            return normalize_provider_value(&candidate, variant, label, depth + 1);
        }
    }
    // No declared variant matches the value's shape; leave the original value
    // alone so full schema validation can produce the authoritative error.
    Ok(value.clone())
}

fn value_shape(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Object(_) => "object",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Null => "null",
    }
}

fn schema_type_matches(schema: &serde_json::Value, expected: &str) -> bool {
    match schema.get("type") {
        Some(serde_json::Value::String(actual)) => actual == expected,
        Some(serde_json::Value::Array(types)) => {
            types.iter().any(|actual| actual.as_str() == Some(expected))
        }
        _ => false,
    }
}

fn is_json_container_string(value: &serde_json::Value) -> bool {
    value
        .as_str()
        .map(str::trim)
        .is_some_and(|text| text.starts_with('{') || text.starts_with('['))
}

fn coerce_json_string(
    value: &serde_json::Value,
    label: &'static str,
) -> Result<serde_json::Value, AgentLoopHostError> {
    let Some(text) = value.as_str() else {
        return Ok(value.clone());
    };
    let trimmed = text.trim();
    if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
        return Ok(value.clone());
    }
    serde_json::from_str(trimmed).map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            format!("{label} could not be parsed as schema-declared JSON"),
        )
    })
}

fn coerce_integer_string(
    value: &serde_json::Value,
    label: &'static str,
) -> Result<serde_json::Value, AgentLoopHostError> {
    let Some(text) = value.as_str() else {
        return Ok(value.clone());
    };
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.contains('.') || trimmed.contains('e') || trimmed.contains('E')
    {
        return Err(provider_coercion_error(label, "integer"));
    }
    let parsed = trimmed
        .parse::<i64>()
        .map_err(|_| provider_coercion_error(label, "integer"))?;
    Ok(serde_json::Value::Number(parsed.into()))
}

fn coerce_number_string(
    value: &serde_json::Value,
    label: &'static str,
) -> Result<serde_json::Value, AgentLoopHostError> {
    let Some(text) = value.as_str() else {
        return Ok(value.clone());
    };
    let parsed = text
        .trim()
        .parse::<f64>()
        .map_err(|_| provider_coercion_error(label, "number"))?;
    let number = serde_json::Number::from_f64(parsed)
        .ok_or_else(|| provider_coercion_error(label, "number"))?;
    Ok(serde_json::Value::Number(number))
}

fn coerce_boolean_string(
    value: &serde_json::Value,
    label: &'static str,
) -> Result<serde_json::Value, AgentLoopHostError> {
    let Some(text) = value.as_str() else {
        return Ok(value.clone());
    };
    match text.trim().to_ascii_lowercase().as_str() {
        "true" => Ok(serde_json::Value::Bool(true)),
        "false" => Ok(serde_json::Value::Bool(false)),
        _ => Err(provider_coercion_error(label, "boolean")),
    }
}

fn provider_coercion_error(label: &'static str, expected: &'static str) -> AgentLoopHostError {
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::InvalidInvocation,
        format!("{label} could not be coerced to schema-declared {expected}"),
    )
}
