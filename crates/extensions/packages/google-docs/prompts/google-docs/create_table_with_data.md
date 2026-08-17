Insert and populate a complete rectangular table, optionally bolding its first row, then verify the provider state.

Use `inspect_document` to choose an insertion index. Prefer this operation over `insert_table` plus individual cell edits.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
