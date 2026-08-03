use serde_json::{Map, Value};

const MAX_PAGE_ITEMS: usize = 100;

pub(crate) fn compact_pull_request_list(response: String) -> Result<String, String> {
    let items: Vec<Value> =
        serde_json::from_str(&response).map_err(|_| invalid_response_error())?;
    validate_page_size(items.len())?;
    let compact = items
        .iter()
        .map(compact_pull_request)
        .collect::<Result<Vec<_>, _>>()?;
    serialize(&compact)
}

pub(crate) fn compact_issue_search(response: String) -> Result<String, String> {
    let response: Value = serde_json::from_str(&response).map_err(|_| invalid_response_error())?;
    let object = response.as_object().ok_or_else(invalid_response_error)?;
    let items = object
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(invalid_response_error)?;
    validate_page_size(items.len())?;

    let mut compact = copy_fields(object, &["total_count", "incomplete_results"]);
    compact.insert(
        "items".to_string(),
        Value::Array(
            items
                .iter()
                .map(compact_search_item)
                .collect::<Result<Vec<_>, _>>()?,
        ),
    );
    serialize(&Value::Object(compact))
}

fn compact_pull_request(item: &Value) -> Result<Value, String> {
    let item = item.as_object().ok_or_else(invalid_response_error)?;
    let mut compact = copy_fields(
        item,
        &[
            "number",
            "title",
            "state",
            "draft",
            "locked",
            "html_url",
            "created_at",
            "updated_at",
            "closed_at",
            "merged_at",
            "author_association",
        ],
    );
    copy_nullable_object(item, &mut compact, "user", &["login"])?;
    copy_object_array(item, &mut compact, "labels", &["name"])?;
    copy_object_array(item, &mut compact, "assignees", &["login"])?;
    copy_object_array(item, &mut compact, "requested_reviewers", &["login"])?;
    copy_object_array(item, &mut compact, "requested_teams", &["slug"])?;
    copy_nullable_object(item, &mut compact, "milestone", &["title"])?;
    copy_object(item, &mut compact, "head", &["ref", "sha"])?;
    copy_object(item, &mut compact, "base", &["ref", "sha"])?;
    Ok(Value::Object(compact))
}

fn compact_search_item(item: &Value) -> Result<Value, String> {
    let item = item.as_object().ok_or_else(invalid_response_error)?;
    let mut compact = copy_fields(
        item,
        &[
            "number",
            "title",
            "state",
            "state_reason",
            "draft",
            "locked",
            "html_url",
            "repository_url",
            "comments",
            "created_at",
            "updated_at",
            "closed_at",
            "author_association",
            "score",
        ],
    );
    copy_nullable_object(item, &mut compact, "user", &["login"])?;
    copy_object_array(item, &mut compact, "labels", &["name"])?;
    copy_object_array(item, &mut compact, "assignees", &["login"])?;
    copy_nullable_object(item, &mut compact, "milestone", &["title"])?;
    copy_nullable_object(
        item,
        &mut compact,
        "pull_request",
        &["url", "html_url", "merged_at"],
    )?;
    Ok(Value::Object(compact))
}

fn copy_fields(source: &Map<String, Value>, fields: &[&str]) -> Map<String, Value> {
    fields
        .iter()
        .filter_map(|field| {
            source
                .get(*field)
                .map(|value| ((*field).to_string(), value.clone()))
        })
        .collect()
}

fn copy_object(
    source: &Map<String, Value>,
    target: &mut Map<String, Value>,
    field: &str,
    fields: &[&str],
) -> Result<(), String> {
    let Some(value) = source.get(field) else {
        return Ok(());
    };
    let object = value.as_object().ok_or_else(invalid_response_error)?;
    target.insert(
        field.to_string(),
        Value::Object(copy_fields(object, fields)),
    );
    Ok(())
}

fn copy_nullable_object(
    source: &Map<String, Value>,
    target: &mut Map<String, Value>,
    field: &str,
    fields: &[&str],
) -> Result<(), String> {
    let Some(value) = source.get(field) else {
        return Ok(());
    };
    if value.is_null() {
        target.insert(field.to_string(), Value::Null);
        return Ok(());
    }
    copy_object(source, target, field, fields)
}

fn copy_object_array(
    source: &Map<String, Value>,
    target: &mut Map<String, Value>,
    field: &str,
    fields: &[&str],
) -> Result<(), String> {
    let Some(value) = source.get(field) else {
        return Ok(());
    };
    let items = value.as_array().ok_or_else(invalid_response_error)?;
    let compact = items
        .iter()
        .map(|item| {
            item.as_object()
                .map(|object| Value::Object(copy_fields(object, fields)))
                .ok_or_else(invalid_response_error)
        })
        .collect::<Result<Vec<_>, _>>()?;
    target.insert(field.to_string(), Value::Array(compact));
    Ok(())
}

fn validate_page_size(item_count: usize) -> Result<(), String> {
    if item_count > MAX_PAGE_ITEMS {
        return Err(invalid_response_error());
    }
    Ok(())
}

fn serialize(value: &impl serde::Serialize) -> Result<String, String> {
    serde_json::to_string(value).map_err(|_| invalid_response_error())
}

fn invalid_response_error() -> String {
    "github_api_invalid_response".to_string()
}
