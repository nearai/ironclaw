Inspect a Google Docs document as structured paragraphs and tables, including indexes and cell contents.

Use this instead of `get_document` when planning indexed edits or working with tables. It returns provider structure in one call; do not create scratch documents to infer indexes.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
