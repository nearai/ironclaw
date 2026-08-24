Execute raw Docs API batchUpdate requests.

Use this low-level escape hatch only when the semantic operations do not cover the edit. Prefer `apply_text_edits` and `create_table_with_data` for their validation and provider read-back.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
