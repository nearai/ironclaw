# google-drive.find_files_compact

Use this for finding likely relevant Drive files without reading file contents.

Prefer this over `google-drive.list_files` when the user only needs candidates, links, or lightweight context. It returns compact file cards: id, name, MIME type, modified time, link, folder/shared/owned flags, and owner.

Use `google-docs.read_excerpt`, `google-sheets.preview`, or file download only after selecting the specific file that needs content.
