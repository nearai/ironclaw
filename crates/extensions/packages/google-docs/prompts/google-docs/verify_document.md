Verify required text fragments and exact row-major table contents against the current Google Docs provider state.

This operation never mutates the document. A mismatch returns `verified: false` with per-expectation checks instead of failing the tool call.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
