Apply multiple text-anchored replacements atomically and read the document back to verify the result.

Anchors must be unique by default. Set `replace_all` only when every occurrence should change. Prefer this over manually calculating indexes for ordinary text revisions.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
