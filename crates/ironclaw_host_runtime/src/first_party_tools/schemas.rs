use serde_json::{Value, json};

pub(crate) fn resolve_builtin_input_schema_ref(reference: &str) -> Option<Value> {
    Some(match reference {
        "schemas/builtin/echo.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "message": { "type": "string", "description": "Message to echo" }
            },
            "required": ["message"],
            "additionalProperties": false
        }),
        "schemas/builtin/time.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["now", "parse", "convert", "format", "diff"],
                    "description": "Time operation to perform. Defaults to now."
                },
                "input": { "type": "string", "description": "Timestamp input for parse, convert, format, or diff" },
                "timestamp": { "type": "string", "description": "Alias for input" },
                "timestamp2": { "type": "string", "description": "Second timestamp for diff" },
                "timezone": { "type": "string", "description": "IANA timezone name" },
                "from_timezone": { "type": "string", "description": "IANA timezone for interpreting the input" },
                "to_timezone": { "type": "string", "description": "IANA timezone for conversion output" },
                "format": { "type": "string", "description": "chrono format string for format operation" },
                "format_string": { "type": "string", "description": "Alias for format" }
            },
            "additionalProperties": false
        }),
        "schemas/builtin/json.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["parse", "stringify", "query", "validate"]
                },
                "data": { "description": "JSON string or JSON value to process" },
                "path": { "type": "string", "description": "Dot/bracket path for query operation" }
            },
            "required": ["operation", "data"],
            "additionalProperties": false
        }),
        "schemas/builtin/http.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "Absolute HTTP or HTTPS URL" },
                "method": {
                    "type": "string",
                    "enum": ["get", "post", "put", "patch", "delete", "head"],
                    "description": "HTTP method. Defaults to get."
                },
                "headers": {
                    "description": "HTTP headers as an object or array of {name,value} entries"
                },
                "body": { "description": "String or JSON request body" },
                "body_base64": { "type": "string", "description": "Base64-encoded request body" },
                "response_body_limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 10485760,
                    "default": 10485760,
                    "description": "Maximum response body bytes. Defaults to 10 MiB; smaller values are raised to 10 MiB."
                },
                "timeout_ms": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 30000,
                    "default": 10000,
                    "description": "Request timeout in milliseconds. Defaults to 10s and is capped at 30s."
                }
            },
            "required": ["url"],
            "additionalProperties": false
        }),
        "schemas/builtin/shell.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Shell command to execute" },
                "workdir": { "type": "string", "description": "Optional scoped working directory" },
                "timeout": { "type": "integer", "minimum": 1, "description": "Timeout in seconds" }
            },
            "required": ["command"],
            "additionalProperties": false
        }),
        "schemas/builtin/read_file.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Scoped path to read" },
                "offset": { "type": "integer", "minimum": 0, "description": "1-based starting line; 0 starts at the beginning" },
                "limit": { "type": "integer", "minimum": 0, "description": "Maximum lines to return" }
            },
            "required": ["path"],
            "additionalProperties": false
        }),
        "schemas/builtin/write_file.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Scoped path to write" },
                "content": { "type": "string", "description": "Complete file content" }
            },
            "required": ["path", "content"],
            "additionalProperties": false
        }),
        "schemas/builtin/list_dir.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Scoped directory path. Defaults to the workspace root." },
                "recursive": { "type": "boolean", "description": "Whether to list recursively" },
                "max_depth": { "type": "integer", "minimum": 0, "description": "Maximum recursive depth" }
            },
            "additionalProperties": false
        }),
        "schemas/builtin/glob.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Glob pattern relative to path" },
                "path": { "type": "string", "description": "Scoped root path. Defaults to the workspace root." },
                "max_results": { "type": "integer", "minimum": 0 }
            },
            "required": ["pattern"],
            "additionalProperties": false
        }),
        "schemas/builtin/grep.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Regular expression to search for" },
                "path": { "type": "string", "description": "Scoped file or directory path. Defaults to the workspace root." },
                "glob": { "type": "string", "description": "Optional glob filter relative to path" },
                "type_filter": { "type": "string", "description": "Optional file type filter" },
                "output_mode": {
                    "type": "string",
                    "enum": ["content", "files_with_matches", "count"],
                    "description": "Output mode. Defaults to files_with_matches."
                },
                "case_insensitive": { "type": "boolean" },
                "multiline": { "type": "boolean" },
                "context": { "type": "integer", "minimum": 0 },
                "before_context": { "type": "integer", "minimum": 0 },
                "after_context": { "type": "integer", "minimum": 0 },
                "head_limit": { "type": "integer", "minimum": 0 },
                "offset": { "type": "integer", "minimum": 0 }
            },
            "required": ["pattern"],
            "additionalProperties": false
        }),
        "schemas/builtin/apply_patch.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Scoped file path to patch" },
                "old_string": { "type": "string", "description": "Exact text to replace" },
                "new_string": { "type": "string", "description": "Replacement text" },
                "replace_all": { "type": "boolean", "description": "Replace every match instead of exactly one" }
            },
            "required": ["path", "old_string", "new_string"],
            "additionalProperties": false
        }),
        "schemas/builtin/skill_list.input.v1.json" => json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
        "schemas/builtin/skill_install.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Optional skill name to use for the installed SKILL.md document"
                },
                "content": {
                    "type": "string",
                    "description": "Raw SKILL.md content to install"
                }
            },
            "required": ["content"],
            "additionalProperties": false
        }),
        "schemas/builtin/skill_install_url.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Optional skill name to use for the installed SKILL.md document"
                },
                "url": {
                    "type": "string",
                    "description": "HTTPS URL to a SKILL.md document, ZIP bundle, or GitHub skill repository/tree to fetch and install"
                }
            },
            "required": ["url"],
            "additionalProperties": false
        }),
        "schemas/builtin/skill_remove.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Name of the installed skill to remove" }
            },
            "required": ["name"],
            "additionalProperties": false
        }),
        _ => return None,
    })
}
