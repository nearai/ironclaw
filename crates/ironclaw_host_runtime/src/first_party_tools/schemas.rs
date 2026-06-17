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
        "schemas/builtin/http.input.v1.json" => http_schema(false),
        "schemas/builtin/http-save.input.v1.json" => http_schema(true),
        "schemas/builtin/memory_search.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Preferred natural language search query for persistent memory"
                },
                "q": {
                    "type": "string",
                    "description": "Alias for query"
                },
                "text": {
                    "type": "string",
                    "description": "Alias for query"
                },
                "pattern": {
                    "type": "string",
                    "description": "Alias for query"
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 20,
                    "default": 5,
                    "description": "Maximum number of memory results to return"
                }
            },
            "required": ["query"],
            "additionalProperties": false
        }),
        "schemas/builtin/memory_write.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "Full content to write or append"
                },
                "target": {
                    "type": "string",
                    "description": "Where to write: 'memory' for MEMORY.md, 'daily_log' for today's log, 'heartbeat' for HEARTBEAT.md checklist, 'bootstrap' to clear BOOTSTRAP.md (content is ignored; the file is always cleared), or a relative memory document path.",
                    "default": "daily_log"
                },
                "append": {
                    "type": "boolean",
                    "description": "Append to existing content when true; replace when false",
                    "default": true
                },
                "metadata": {
                    "type": "object",
                    "description": "Optional document metadata such as skip_indexing or skip_versioning"
                },
                "old_string": {
                    "type": "string",
                    "description": "Exact text to replace; switches to patch mode"
                },
                "new_string": {
                    "type": "string",
                    "description": "Replacement text for patch mode"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace every old_string occurrence in patch mode",
                    "default": false
                },
                "timezone": {
                    "type": "string",
                    "description": "IANA timezone used only for daily_log target date resolution"
                }
            },
            "additionalProperties": false
        }),
        "schemas/builtin/memory_read.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Relative memory document path to read"
                }
            },
            "required": ["path"],
            "additionalProperties": false
        }),
        "schemas/builtin/memory_tree.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Relative memory directory path to list; omit for the memory root",
                    "default": ""
                },
                "depth": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 10,
                    "default": 1,
                    "description": "Maximum directory depth to include"
                }
            },
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
        // NOTE: this schema is published by the host_runtime first-party
        // capability registry (consumed by `surface.rs::resolve_builtin_input_schema_ref`).
        // The decorator path (`ironclaw_loop_support::build_spawn_subagent_parameters_schema`)
        // builds an equivalent schema dynamically from the registered flavor
        // catalog and overrides the model-facing tool definition at runtime.
        // The two shapes MUST stay in sync. Long-term, route this entry
        // through the canonical builder to eliminate the dual source of truth.
        "schemas/builtin/spawn_subagent.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "subagent_type": {
                    "type": "string",
                    "enum": ["general", "explorer", "coder", "planner"],
                    "description": "Which subagent profile to spawn. Options:\n- general: read-only file exploration (read_file, list_dir, grep)\n- explorer: read + glob over filesystem (read_file, list_dir, grep, glob)\n- coder: read + write + shell (read_file, write_file, apply_patch, shell, list_dir, grep, glob)\n- planner: read codebase + web research, returns a structured implementation plan (read_file, list_dir, grep, glob, http)"
                },
                "task": {
                    "type": "string",
                    "description": "Task for the child subagent run"
                },
                "handoff": {
                    "type": "string",
                    "description": "Optional context to pass to the child subagent"
                }
            },
            "required": ["subagent_type", "task"],
            "additionalProperties": false
        }),
        "schemas/builtin/trace_commons-onboard.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "invite_url": {
                    "type": "string",
                    "description": "Trace Commons operator-issued invite link (https://…/onboard#CODE)"
                },
                "include_message_text": {
                    "type": "boolean",
                    "description": "Whether contributions may include redacted message text (default: false)"
                },
                "include_tool_payloads": {
                    "type": "boolean",
                    "description": "Whether contributions may include redacted tool payloads (default: false)"
                },
                "confirmed": {
                    "type": "boolean",
                    "description": "Must be true only after the user has explicitly consented in this conversation (default: false)"
                }
            },
            "required": ["invite_url"],
            "additionalProperties": false
        }),
        "schemas/builtin/trace_commons-status.input.v1.json" => json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
        "schemas/builtin/trace_commons-credits.input.v1.json" => json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
        "schemas/builtin/trace_commons-profile_token.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "confirmed": {
                    "type": "boolean",
                    "description": "Must be true only after the user has explicitly asked to mint a manual/browser profile-management token in this conversation (default: false)"
                }
            },
            "additionalProperties": false
        }),
        "schemas/builtin/trace_commons-profile_set.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "display_handle": {
                    "type": "string",
                    "description": "Pseudonymous public display handle, 3-32 ASCII letters, digits, '-' or '_'"
                },
                "bio": {
                    "type": "string",
                    "description": "Optional short public bio, at most 280 bytes"
                },
                "confirmed": {
                    "type": "boolean",
                    "description": "Must be true only after the user has explicitly approved publishing this handle/bio in this conversation (default: false)"
                }
            },
            "required": ["display_handle"],
            "additionalProperties": false
        }),
        "schemas/builtin/profile_set.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "timezone": {
                    "type": "string",
                    "description": "IANA timezone name, e.g. America/Los_Angeles or Asia/Tokyo"
                },
                "locale": {
                    "type": "string",
                    "description": "BCP-47 locale tag, e.g. en-US or ja-JP",
                    "maxLength": 35
                },
                "location": {
                    "type": "string",
                    "description": "Free-text location label, e.g. Tokyo, Japan"
                }
            },
            "minProperties": 1,
            "additionalProperties": false
        }),
        "schemas/builtin/read_file.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Scoped path to read. Supported document files such as PDFs are returned as extracted text." },
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
        "schemas/builtin/extension_search.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Optional extension, product, provider, or service name to search in the local Reborn extension catalog. Omit to list bundled and installed extensions." }
            },
            "additionalProperties": false
        }),
        "schemas/builtin/extension_install.input.v1.json"
        | "schemas/builtin/extension_activate.input.v1.json"
        | "schemas/builtin/extension_remove.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "extension_id": { "type": "string", "description": "Extension id from extension_search results" }
            },
            "required": ["extension_id"],
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
                    "description": "Raw SKILL.md content to install, or plain Markdown when name is provided"
                },
                "url": {
                    "type": "string",
                    "description": "HTTPS URL to a SKILL.md document, ZIP bundle, or GitHub skill repository/tree to fetch and install"
                }
            },
            "oneOf": [
                { "required": ["content"] },
                { "required": ["url"] }
            ],
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
        "schemas/builtin/trigger_create.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Human-readable trigger name. Runtime validation caps UTF-8 content at 256 bytes."
                },
                "prompt": {
                    "type": "string",
                    "description": "Prompt submitted when the trigger fires. Runtime validation caps UTF-8 content at 32768 bytes. Do not embed delivery routing here; when the user asks to send routine or trigger results through an outbound product/channel, first select the target through the visible outbound delivery target capabilities, then create the trigger."
                },
                "cron": { "type": "string", "description": "Five-, six-, or seven-field cron expression; fire cadence must be at least one minute" },
                "timezone": { "type": "string", "description": "IANA timezone name for cron evaluation (e.g. 'America/New_York', 'Europe/London', 'UTC'). The cron expression is evaluated in this timezone; fire times are stored and compared in UTC. If the user's timezone is already known from the conversation or their settings, use it without asking; if unknown, ask the user before creating the trigger. Never silently assume UTC — a trigger that fires at the wrong local time is worse than no trigger." }
            },
            "required": ["name", "prompt", "cron", "timezone"],
            "additionalProperties": false
        }),
        "schemas/builtin/trigger_list.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "limit": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 100,
                    "description": "Maximum triggers to return. Defaults to 100."
                },
                "run_limit": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 100,
                    "description": "Maximum recent runs to embed per trigger. Defaults to 25."
                }
            },
            "additionalProperties": false
        }),
        "schemas/builtin/trigger_remove.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "trigger_id": { "type": "string", "description": "Trigger id returned by trigger_create or trigger_list" }
            },
            "required": ["trigger_id"],
            "additionalProperties": false
        }),
        _ => return None,
    })
}

fn http_schema(require_save_to: bool) -> Value {
    let mut properties = json!({
        "url": { "type": "string", "description": "Absolute HTTP or HTTPS URL" },
        "method": {
            "type": "string",
            "enum": ["get", "post", "put", "patch", "delete", "head"],
            "description": "HTTP method. Defaults to get."
        },
        "headers": {
            "description": "HTTP headers as an object or array of {name,value} entries",
            "oneOf": [
                {
                    "type": "object",
                    "additionalProperties": { "type": "string" }
                },
                {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" },
                            "value": { "type": "string" }
                        },
                        "required": ["name", "value"],
                        "additionalProperties": false
                    }
                }
            ]
        },
        "body": {
            "description": "String or JSON request body",
            "type": ["string", "object", "array", "number", "boolean", "null"]
        },
        "body_base64": { "type": "string", "description": "Base64-encoded request body" },
        "response_body_limit": response_body_limit_schema(require_save_to),
        "timeout_ms": {
            "type": "integer",
            "minimum": 1,
            "maximum": 30000,
            "default": 10000,
            "description": "Request timeout in milliseconds. Defaults to 10s and is capped at 30s."
        }
    });
    let mut required = vec!["url"];
    if require_save_to {
        properties["save_to"] = json!({
            "type": "string",
            "description": "Scoped path to save the sanitized response body for builtin.http.save instead of inlining body data, e.g. /workspace/response.json"
        });
        required.push("save_to");
    }

    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

fn response_body_limit_schema(require_save_to: bool) -> Value {
    let default = if require_save_to { 10_485_760 } else { 49_152 };
    let maximum = if require_save_to { 10_485_760 } else { 262_144 };
    let description = if require_save_to {
        "Maximum sanitized response body bytes to fetch and save. Defaults to 10 MiB; smaller values are honored."
    } else {
        "Maximum inline response body bytes exposed to the model. Defaults to a small model-visible budget and is capped at 256 KiB; smaller values are honored, and oversized bodies are truncated or summarized with guidance to use builtin.http.save."
    };
    json!({
        "type": "integer",
        "minimum": 1,
        "maximum": maximum,
        "default": default,
        "description": description
    })
}
