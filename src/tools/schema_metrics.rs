fn schema_is_typed_property(schema: &serde_json::Value) -> bool {
    matches!(
        schema.get("type"),
        Some(serde_json::Value::String(_)) | Some(serde_json::Value::Array(_))
    ) || schema.get("$ref").is_some()
        || schema.get("anyOf").is_some()
        || schema.get("oneOf").is_some()
        || schema.get("allOf").is_some()
        || schema.get("items").is_some()
        || schema
            .get("properties")
            .and_then(|p| p.as_object())
            .is_some()
        || schema
            .get("additionalProperties")
            .is_some_and(serde_json::Value::is_object)
}

pub(crate) fn is_permissive_schema(schema: &serde_json::Value) -> bool {
    if schema
        .get("properties")
        .and_then(|p| p.as_object())
        .is_some_and(|p| !p.is_empty())
    {
        return false;
    }

    for key in ["oneOf", "anyOf", "allOf"] {
        if let Some(variants) = schema.get(key).and_then(|v| v.as_array())
            && variants.iter().any(|variant| {
                variant
                    .get("properties")
                    .and_then(|p| p.as_object())
                    .is_some_and(|p| !p.is_empty())
            })
        {
            return false;
        }
    }

    true
}

pub(crate) fn typed_property_count(schema: &serde_json::Value) -> usize {
    let mut all_props = serde_json::Map::new();

    if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
        all_props.extend(props.iter().map(|(k, v)| (k.clone(), v.clone())));
    }

    for key in ["allOf", "oneOf", "anyOf"] {
        if let Some(variants) = schema.get(key).and_then(|v| v.as_array()) {
            for variant in variants {
                if let Some(props) = variant.get("properties").and_then(|p| p.as_object()) {
                    all_props.extend(props.iter().map(|(k, v)| (k.clone(), v.clone())));
                }
            }
        }
    }

    all_props
        .values()
        .filter(|prop| schema_is_typed_property(prop))
        .count()
}
