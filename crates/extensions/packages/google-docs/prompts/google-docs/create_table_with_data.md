Insert and populate a complete rectangular table, optionally bolding its first row, then verify the provider state.

Use `inspect_document` to choose an insertion index. Prefer this operation over `insert_table` plus individual cell edits.

Check `verified` before reporting success. If it is false, `stage` and `failure` describe the last committed stage; inspect provider state before retrying so a partial write does not create a duplicate table.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
