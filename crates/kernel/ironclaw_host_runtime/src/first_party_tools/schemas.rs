use serde_json::{Value, json};

use crate::first_party_tools::time::UNIX_MILLIS_THRESHOLD;

/// Input schemas for the native memory extension (`ironclaw.memory`),
/// served inline by `surface.rs` the way builtin schemas are.
///
/// The bundled v3 manifest's asset schema files are the single source of truth:
/// they are compiled in here and parsed, rather than materialized to the
/// filesystem, because native memory rides the always-on lane. `document-{read,
/// write}`, `search`, and `tree` each reference their own bundled schema file.
pub(crate) fn resolve_native_memory_input_schema_ref(reference: &str) -> Option<Value> {
    let raw = match reference {
        "schemas/memory/document-read.input.v1.json" => {
            include_str!(
                "../../../../extensions/packages/memory-native/schemas/memory/document-read.input.v1.json"
            )
        }
        "schemas/memory/document-write.input.v1.json" => {
            include_str!(
                "../../../../extensions/packages/memory-native/schemas/memory/document-write.input.v1.json"
            )
        }
        "schemas/memory/search.input.v1.json" => {
            include_str!(
                "../../../../extensions/packages/memory-native/schemas/memory/search.input.v1.json"
            )
        }
        "schemas/memory/tree.input.v1.json" => {
            include_str!(
                "../../../../extensions/packages/memory-native/schemas/memory/tree.input.v1.json"
            )
        }
        "schemas/memory/profile-set.input.v1.json" => {
            include_str!(
                "../../../../extensions/packages/memory-native/schemas/memory/profile-set.input.v1.json"
            )
        }
        _ => return None,
    };
    // silent-ok: these are compile-embedded assets validated by the
    // `memory_native_schema_validation` test; a malformed schema fails that test
    // and the build, so `.ok()` here cannot silently mask a real fault.
    serde_json::from_str(raw).ok()
}

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
                "input": timestamp_input_schema("Timestamp input for parse, convert, format, or diff"),
                "timestamp": timestamp_input_schema("Alias for input"),
                "timestamp2": timestamp_input_schema("Second timestamp for diff"),
                "timezone": { "type": "string", "description": "IANA timezone name" },
                "utc_offset": { "type": "string", "description": "UTC offset for now output, e.g. +03:00 or -07:00" },
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
                    "enum": ["parse", "stringify", "query", "validate", "length", "last", "slice", "aggregate"],
                    "description": "JSON operation to perform"
                },
                "data": { "description": "JSON string or JSON value to process" },
                "file_path": {
                    "type": "string",
                    "maxLength": 4096,
                    "description": "Scoped JSON file below /workspace to read for query or collection operations; mutually exclusive with data and bounded to 8388608 bytes"
                },
                "path": {
                    "type": "string",
                    "maxLength": 4096,
                    "description": "Restricted dot/bracket traversal path with an optional $ root, including $, $[1][0], $.nodes[2].data[15][0], [1][0], or nodes[2].data[15][0]. Wildcards, negative indices, slices, filters, recursive descent, expressions, and functions are not supported; use length, last, slice, or aggregate operations instead."
                },
                "start": { "type": "integer", "minimum": 0, "description": "Inclusive array start index for slice" },
                "end": { "type": "integer", "minimum": 0, "description": "Exclusive array end index for slice; at most 4096 items may be returned" },
                "function": { "type": "string", "enum": ["sum", "average", "min", "max"], "description": "Bounded numeric aggregate: integer-only input computes exactly (sum, min, max) and average rounds its exact sum once; input mixing integers and decimals computes in floating point" },
                "value_index": { "type": "integer", "minimum": 0, "description": "Optional numeric index to select from each array row before aggregation" }
            },
            "oneOf": [
                {
                    "title": "parse inline JSON text",
                    "properties": {
                        "operation": { "const": "parse" },
                        "data": { "type": "string" }
                    },
                    "required": ["operation", "data"],
                    "not": { "anyOf": [{ "required": ["file_path"] }, { "required": ["path"] }] }
                },
                {
                    "title": "stringify inline JSON",
                    "properties": { "operation": { "const": "stringify" } },
                    "required": ["operation", "data"],
                    "not": { "anyOf": [{ "required": ["file_path"] }, { "required": ["path"] }] }
                },
                {
                    "title": "query inline JSON",
                    "properties": { "operation": { "const": "query" } },
                    "required": ["operation", "data", "path"],
                    "not": { "required": ["file_path"] }
                },
                {
                    "title": "query a scoped workspace JSON file",
                    "properties": { "operation": { "const": "query" } },
                    "required": ["operation", "file_path", "path"],
                    "not": { "required": ["data"] }
                },
                {
                    "title": "get inline JSON collection length",
                    "properties": { "operation": { "const": "length" } },
                    "required": ["operation", "data", "path"],
                    "not": { "required": ["file_path"] }
                },
                {
                    "title": "get workspace JSON collection length",
                    "properties": { "operation": { "const": "length" } },
                    "required": ["operation", "file_path", "path"],
                    "not": { "required": ["data"] }
                },
                {
                    "title": "get last inline JSON array item",
                    "properties": { "operation": { "const": "last" } },
                    "required": ["operation", "data", "path"],
                    "not": { "required": ["file_path"] }
                },
                {
                    "title": "get last workspace JSON array item",
                    "properties": { "operation": { "const": "last" } },
                    "required": ["operation", "file_path", "path"],
                    "not": { "required": ["data"] }
                },
                {
                    "title": "slice an inline JSON array",
                    "properties": { "operation": { "const": "slice" } },
                    "required": ["operation", "data", "path", "start", "end"],
                    "not": { "required": ["file_path"] }
                },
                {
                    "title": "slice a workspace JSON array",
                    "properties": { "operation": { "const": "slice" } },
                    "required": ["operation", "file_path", "path", "start", "end"],
                    "not": { "required": ["data"] }
                },
                {
                    "title": "aggregate an inline JSON numeric array",
                    "properties": { "operation": { "const": "aggregate" } },
                    "required": ["operation", "data", "path", "function"],
                    "not": { "required": ["file_path"] }
                },
                {
                    "title": "aggregate a workspace JSON numeric array",
                    "properties": { "operation": { "const": "aggregate" } },
                    "required": ["operation", "file_path", "path", "function"],
                    "not": { "required": ["data"] }
                },
                {
                    "title": "validate inline JSON text",
                    "properties": {
                        "operation": { "const": "validate" },
                        "data": { "type": "string" }
                    },
                    "required": ["operation", "data"],
                    "not": { "anyOf": [{ "required": ["file_path"] }, { "required": ["path"] }] }
                }
            ],
            "additionalProperties": false
        }),
        "schemas/builtin/http.input.v1.json" => http_schema(false),
        "schemas/builtin/outbound_deliver.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "target_id": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 512,
                    "description": "Opaque target id returned by builtin__outbound_delivery_targets_list."
                },
                "content": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 32768,
                    "description": "Markdown content to deliver. The channel renders and splits it. The authoritative limit is 32768 UTF-8 BYTES (schema maxLength counts code points, so multi-byte content may pass the schema and still be rejected)."
                }
            },
            "required": ["target_id", "content"],
            "additionalProperties": false
        }),
        "schemas/builtin/outbound_deliver.output.v1.json" => json!({
            "type": "object",
            "properties": {
                "delivered": { "type": "boolean" },
                "provider_confirmed": { "type": "boolean" },
                "target_id": { "type": "string" },
                "channel": { "type": "string" },
                "display_name": { "type": "string" },
                "provider_message_refs": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "durably_recorded": { "type": "boolean" },
                "already_delivered": {
                    "type": "boolean",
                    "description": "True when the durable ledger proves this exact delivery already completed earlier in the run. The message was NOT resent; provider_message_refs is empty because the ledger row does not retain them."
                }
            },
            "required": ["delivered", "provider_confirmed", "target_id", "channel", "display_name", "provider_message_refs", "durably_recorded", "already_delivered"],
            "additionalProperties": false
        }),
        "schemas/builtin/attach_workspace_file_to_reply.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Existing file path under /workspace to attach to the final reply."
                },
                "filename": {
                    "type": "string",
                    "description": "Optional safe download filename. Defaults to the workspace basename."
                },
                "mime_type": {
                    "type": "string",
                    "description": "Optional MIME type override. Defaults from common filename extensions or application/octet-stream."
                }
            },
            "required": ["path"],
            "additionalProperties": false
        }),
        "schemas/builtin/attach_workspace_file_to_reply.output.v1.json" => json!({
            "type": "object",
            "properties": {
                "attached": { "type": "boolean", "const": true },
                "attachment_ref": {
                    "type": "string",
                    "pattern": "^att_[0-9a-f]{64}$",
                    "description": "Opaque host-issued reference for the registered attachment."
                },
                "filename": { "type": "string" },
                "mime_type": { "type": "string" },
                "size_bytes": { "type": "integer", "minimum": 0 }
            },
            "required": ["attached", "attachment_ref", "filename", "mime_type", "size_bytes"],
            "additionalProperties": false
        }),
        "schemas/builtin/http-save.input.v1.json" => http_schema(true),
        "schemas/builtin/shell.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Shell command to execute. Prefer ONE command that does the whole job: combine steps with '&&' or pipes, or write and run a single script (awk/python) — do NOT issue one command per metric/day/line, and don't re-read files you already have." },
                "workdir": { "type": "string", "description": "Optional scoped working directory" },
                "timeout": { "type": "integer", "minimum": 1, "description": "Timeout in seconds" }
            },
            "required": ["command"],
            "additionalProperties": false
        }),
        // NOTE: this schema is published by the host_runtime first-party
        // capability registry (consumed by `surface.rs::resolve_builtin_input_schema_ref`).
        // The decorator path (`ironclaw_loop_host::build_spawn_subagent_parameters_schema`)
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
        "schemas/builtin/trace_commons-account_login_link.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "confirmed": {
                    "type": "boolean",
                    "description": "Must be true only after the user explicitly asked to open a Trace Commons account/profile login link in this conversation (default: false)"
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
                "old_string": {
                    "type": ["string", "null"],
                    "description": "Text to replace for a single targeted edit. Exact matches are preferred; fuzzy Unicode and trailing-whitespace normalization is used when exact text is not present."
                },
                "new_string": { "type": ["string", "null"], "description": "Replacement text for a single targeted edit" },
                "edits": {
                    "description": "One or more targeted replacements matched against the original file. Prefer this for multiple disjoint edits.",
                    "oneOf": [
                        {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": 256,
                            "items": {
                                "type": "object",
                                "properties": {
                                    "old_string": { "type": "string", "description": "Text to replace" },
                                    "new_string": { "type": "string", "description": "Replacement text" }
                                },
                                "required": ["old_string", "new_string"],
                                "additionalProperties": false
                            }
                        },
                        { "type": "null" },
                        { "const": "null" }
                    ]
                },
                "replace_all": { "type": "boolean", "description": "Replace every match instead of exactly one. Only valid with a single targeted edit." }
            },
            "required": ["path"],
            "oneOf": [
                {
                    "properties": {
                        "old_string": {
                            "type": "string",
                            "not": { "const": "null" }
                        },
                        "new_string": {
                            "type": "string",
                            "not": { "const": "null" }
                        }
                    },
                    "required": ["old_string", "new_string"],
                    "not": {
                        "properties": {
                            "edits": { "type": "array" }
                        },
                        "required": ["edits"]
                    }
                },
                {
                    "properties": {
                        "edits": { "type": "array" },
                        "old_string": { "enum": ["null", null] },
                        "new_string": { "enum": ["null", null] }
                    },
                    "required": ["edits"]
                }
            ],
            "allOf": [
                {
                    "if": {
                        "properties": {
                            "replace_all": { "const": true }
                        },
                        "required": ["replace_all"]
                    },
                    "then": {
                        "properties": {
                            "edits": {
                                "oneOf": [
                                    {
                                        "type": "array",
                                        "maxItems": 1
                                    },
                                    { "type": "null" },
                                    { "const": "null" }
                                ]
                            }
                        }
                    }
                }
            ],
            "additionalProperties": false
        }),
        "schemas/builtin/extension_search.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Optional extension, product, provider, or service name to search in the local Reborn extension catalog. Omit to list bundled and installed extensions." }
            },
            "additionalProperties": false
        }),
        "schemas/builtin/extension_register_hosted_mcp.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "desired_id": {
                    "type": "string",
                    "description": "Short stable id for this custom MCP server, without the reserved mcp- prefix."
                },
                "desired_name": {
                    "type": "string",
                    "description": "User-facing name for the custom MCP extension."
                },
                "endpoint": {
                    "type": "string",
                    "description": "HTTPS MCP server endpoint supplied by the user or provider documentation."
                },
                "auth_type": {
                    "type": "string",
                    "enum": ["no_auth", "bearer", "oauth"],
                    "description": "Explicit authentication type from provider documentation or user context. Use no_auth only for a documented public endpoint, bearer for an API token or PAT, and oauth for a browser authorization-code flow. Ask the user when unclear; never infer automatically."
                }
            },
            "required": ["desired_id", "desired_name", "endpoint", "auth_type"],
            "additionalProperties": false
        }),
        "schemas/builtin/extension_install.input.v1.json"
        | "schemas/builtin/extension_remove.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "extension_id": { "type": "string", "description": "Extension id from extension_search results" }
            },
            "required": ["extension_id"],
            "additionalProperties": false
        }),
        "schemas/builtin/ironhub_search.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Optional free-text filter matched against entry names and descriptions. OMIT IT to return the entire catalog — that is how you list everything available. A query returns only matching entries: compare total_entries against catalog_total before describing the result as what is available." }
            },
            "additionalProperties": false
        }),
        "schemas/builtin/ironhub_info.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "IronHub tool or skill name." },
                "kind": { "type": "string", "enum": ["tool", "skill"] }
            },
            "required": ["name"],
            "additionalProperties": false
        }),
        "schemas/builtin/ironhub_install.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "IronHub tool or skill name." },
                "kind": { "type": "string", "enum": ["tool", "skill"] },
                "force": { "type": "boolean", "default": false },
                "expected_version": { "type": "string" },
                "expected_artifact_digest": { "type": "string" }
            },
            "required": ["name"],
            "additionalProperties": false
        }),
        "schemas/builtin/ironhub_search.output.v1.json"
        | "schemas/builtin/ironhub_info.output.v1.json"
        | "schemas/builtin/ironhub_install.output.v1.json" => json!({
            "type": "object",
            "properties": {
                "phase": { "type": "string", "enum": ["discovered", "installed"] },
                "total_entries": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "How many catalog entries MATCHED the request. With a query this is the size of the match, not the catalog."
                },
                "returned_entries": { "type": "integer", "minimum": 0 },
                "catalog_total": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Total entries in the signed catalog, ignoring any query filter. When it exceeds total_entries the result is a filtered subset — say so rather than presenting it as everything available."
                },
                "truncated": {
                    "type": "boolean",
                    "description": "True when entries is an incomplete prefix of the matching signed catalog. Never infer absence from an incomplete result."
                },
                "entries": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "kind": { "type": "string", "enum": ["tool", "skill"] },
                            "name": { "type": "string" },
                            "version": { "type": "string" },
                            "description": { "type": "string" },
                            "provenance": {
                                "type": "string",
                                "enum": ["official", "trusted", "verified", "new"]
                            },
                            "artifact_digest": { "type": "string" }
                        },
                        "required": ["kind", "name", "version", "description", "provenance"],
                        "additionalProperties": false
                    }
                },
                "lifecycle": { "type": "object" },
                "message": { "type": "string" }
            },
            "required": [
                "phase",
                "total_entries",
                "returned_entries",
                "catalog_total",
                "truncated",
                "entries"
            ],
            "additionalProperties": false
        }),
        "schemas/builtin/admin_configuration_replace.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "group_id": {
                    "type": "string",
                    "maxLength": 128,
                    "description": "Manifest-declared administrator configuration group id"
                },
                "expected_revision": { "type": "integer", "minimum": 0 },
                "values": {
                    "type": "array",
                    "maxItems": 64,
                    "items": {
                        "type": "object",
                        "properties": {
                            "handle": { "type": "string", "maxLength": 128 },
                            "value": { "type": "string", "maxLength": 16384 }
                        },
                        "required": ["handle", "value"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["group_id", "expected_revision", "values"],
            "additionalProperties": false
        }),
        "schemas/builtin/admin_configuration_replace.output.v1.json" => json!({
            "type": "object",
            "properties": {
                "group_id": { "type": "string", "maxLength": 128 },
                "revision": { "type": "integer", "minimum": 0 },
                "complete": { "type": "boolean" },
                "fields": {
                    "type": "array",
                    "maxItems": 64,
                    "items": {
                        "type": "object",
                        "properties": {
                            "handle": { "type": "string", "maxLength": 128 },
                            "secret": { "type": "boolean" },
                            "required": { "type": "boolean" },
                            "provided": { "type": "boolean" },
                            "value": { "type": ["string", "null"], "maxLength": 16384 }
                        },
                        "required": ["handle", "secret", "required", "provided", "value"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["group_id", "revision", "complete", "fields"],
            "additionalProperties": false
        }),
        "schemas/builtin/operator_config_set_auto_approve.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "enabled": { "type": "boolean" }
            },
            "required": ["enabled"],
            "additionalProperties": false
        }),
        "schemas/builtin/operator_config_set_auto_approve.output.v1.json" => json!({
            "type": "object",
            "properties": {
                "key": { "const": "agent.auto_approve_tools" },
                "enabled": { "type": "boolean" },
                "tenant_id": { "type": "string", "maxLength": 128 },
                "user_id": { "type": "string", "maxLength": 128 }
            },
            "required": ["key", "enabled", "tenant_id", "user_id"],
            "additionalProperties": false
        }),
        "schemas/builtin/operator_config_set_tool_permission.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "capability_id": { "type": "string", "maxLength": 512 },
                "state": {
                    "type": "string",
                    "enum": ["default", "always_allow", "ask_each_time", "disabled"]
                }
            },
            "required": ["capability_id", "state"],
            "additionalProperties": false
        }),
        "schemas/builtin/operator_config_set_tool_permission.output.v1.json" => json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "maxLength": 517 },
                "capability_id": { "type": "string", "maxLength": 512 },
                "state": {
                    "type": "string",
                    "enum": ["default", "always_allow", "ask_each_time", "disabled"]
                },
                "tenant_id": { "type": "string", "maxLength": 128 },
                "user_id": { "type": "string", "maxLength": 128 }
            },
            "required": ["key", "capability_id", "state", "tenant_id", "user_id"],
            "additionalProperties": false
        }),
        "schemas/builtin/skill_list.input.v1.json" => json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
        "schemas/builtin/skill_list.output.v1.json" => json!({
            "type": "object",
            "properties": {
                "skills": { "type": "array" },
                "count": { "type": "integer", "minimum": 0 }
            },
            "required": ["skills", "count"],
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
                },
                "files": {
                    "type": "array",
                    "description": "Additional bundle files installed alongside SKILL.md, e.g. `scripts/analyze.py`, `references/units.md`, `schemas/report.xsd`. Put a reusable computation in a script rather than describing it in prose: a future run loads the file instead of re-deriving the method. SKILL.md should name the files it relies on.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": "Bundle-relative path, e.g. `scripts/analyze.py`"
                            },
                            "text": {
                                "type": "string",
                                "description": "UTF-8 file contents. Use this for scripts, references and schemas."
                            },
                            "bytes_base64": {
                                "type": "string",
                                "description": "Base64 contents. Only for binary files; prefer `text` otherwise."
                            }
                        },
                        "required": ["path"],
                        "additionalProperties": false
                    }
                }
            },
            "oneOf": [
                { "required": ["content"] },
                { "required": ["url"] }
            ],
            "additionalProperties": false
        }),
        "schemas/builtin/skill_install.output.v1.json" => json!({
            "type": "object",
            "properties": {
                "installed": { "type": "boolean" },
                "name": { "type": "string" },
                "path": { "type": "string" },
                "source": { "type": "string" },
                "files_installed": { "type": "integer", "minimum": 0 }
            },
            "required": ["installed", "name", "path", "source", "files_installed"],
            "additionalProperties": false
        }),
        "schemas/builtin/skill_update.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Name of the installed skill to update"
                },
                "content": {
                    "type": "string",
                    "description": "Replacement SKILL.md content"
                }
            },
            "required": ["name", "content"],
            "additionalProperties": false
        }),
        "schemas/builtin/skill_update.output.v1.json" => json!({
            "type": "object",
            "properties": {
                "updated": { "type": "boolean" },
                "name": { "type": "string" }
            },
            "required": ["updated", "name"],
            "additionalProperties": false
        }),
        "schemas/builtin/skill_auto_activate_set.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Name of the installed skill to update"
                },
                "enabled": {
                    "type": "boolean",
                    "description": "Whether criteria-based activation is enabled for this skill"
                }
            },
            "required": ["name", "enabled"],
            "additionalProperties": false
        }),
        "schemas/builtin/skill_auto_activate_set.output.v1.json" => json!({
            "type": "object",
            "properties": {
                "updated": { "type": "boolean" },
                "name": { "type": "string" },
                "auto_activate": { "type": "boolean" }
            },
            "required": ["updated", "name", "auto_activate"],
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
        "schemas/builtin/skill_remove.output.v1.json" => json!({
            "type": "object",
            "properties": {
                "removed": { "type": "boolean" },
                "name": { "type": "string" }
            },
            "required": ["removed", "name"],
            "additionalProperties": false
        }),
        "schemas/builtin/skill_auto_activate_learned_set.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "enabled": {
                    "type": "boolean",
                    "description": "Whether criteria-based learned skill activation is enabled by default"
                }
            },
            "required": ["enabled"],
            "additionalProperties": false
        }),
        "schemas/builtin/skill_auto_activate_learned_set.output.v1.json" => json!({
            "type": "object",
            "properties": {
                "success": { "type": "boolean" },
                "message": { "type": "string" }
            },
            "required": ["success", "message"],
            "additionalProperties": false
        }),
        "schemas/builtin/trigger_create.input.v1.json" => json!({
            "type": "object",
            "description": "Create a scheduled trigger. Pass the trigger object itself with top-level fields `name`, `prompt`, and `schedule`; do not wrap the schedule in `operation`, `data`, or a parser request object.",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Human-readable trigger name. Runtime validation caps UTF-8 content at 256 bytes."
                },
                "prompt": {
                    "type": "string",
                    "description": "Prompt submitted when the trigger fires, written for a future run with no memory of this conversation. Write the whole task, including any delivery the user wants: name the destination in an explicit step by pinned target id from builtin__outbound_delivery_targets_list, picked while the user is present (for example \"then deliver the summary with builtin__outbound_deliver to chat:team-dm\" — never a description a fire would have to look up). A bare \"send me\" means the surface the user is asking from — never ask which channel: from a channel conversation, default to the channel this conversation is on and pin its target id; from the web app with no external destination named there is no delivery step and no web-app target: the fire's final reply IS the delivery (the run thread records it), so end the prompt with the reply itself and write no delivery step. When the user names an external destination (\"send me this in my messaging app\"), that IS a delivery step even from the web app: pin its target id and write the builtin__outbound_deliver step — reaching the user or anyone else on an external surface always goes through builtin__outbound_deliver, never through integration messaging tools, which act as the user. Each fire's final reply is recorded in the routine's own run thread automatically, so a fire that makes no delivery call delivers nothing externally. Do not describe creating, scheduling, or configuring the trigger. Runtime validation caps UTF-8 content at 32768 bytes."
                },
                "schedule": {
                    "description": "When and how often the trigger fires. This value is the schedule object itself. For recurring triggers use {\"kind\":\"cron\",\"expression\":\"0 14 * * 2\",\"timezone\":\"America/Los_Angeles\"}. For one-time triggers use {\"kind\":\"once\",\"at\":\"2026-06-23T14:00:00\",\"timezone\":\"America/Los_Angeles\"}. Do not pass {\"operation\":\"parse\",\"data\":...}.",
                    "oneOf": [
                        {
                            "type": "object",
                            "properties": {
                                "kind": { "const": "cron" },
                                "expression": { "type": "string", "description": "Five-, six-, or seven-field cron expression; cadence at least one minute. Example: `0 14 * * 2` for Tuesdays at 2 PM in `timezone`." },
                                "timezone": { "type": "string", "description": "IANA timezone name (e.g. America/New_York, UTC)." }
                            },
                            "required": ["kind", "expression", "timezone"],
                            "additionalProperties": false
                        },
                        {
                            "type": "object",
                            "properties": {
                                "kind": { "const": "once" },
                                "at": { "type": "string", "description": "Local wall-clock datetime in `timezone`, format YYYY-MM-DDTHH:MM:SS; interpreted in the given timezone and converted to UTC." },
                                "timezone": { "type": "string", "description": "IANA timezone name (e.g. America/New_York, UTC)." }
                            },
                            "required": ["kind", "at", "timezone"],
                            "additionalProperties": false
                        }
                    ]
                }
            },
            "required": ["name", "prompt", "schedule"],
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
        "schemas/builtin/trigger_pause.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "trigger_id": { "type": "string", "description": "Trigger id returned by trigger_create or trigger_list" }
            },
            "required": ["trigger_id"],
            "additionalProperties": false
        }),
        "schemas/builtin/trigger_resume.input.v1.json" => json!({
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

fn timestamp_input_schema(description: &str) -> Value {
    json!({
        "description": format!(
            "{description}. Accepts an ISO 8601 string, Unix seconds (including fractional Slack timestamps), or Unix milliseconds. Integer values with absolute magnitude at least {UNIX_MILLIS_THRESHOLD} are interpreted as milliseconds."
        ),
        "oneOf": [
            { "type": "string" },
            { "type": "number" }
        ]
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

#[cfg(test)]
mod tests {
    use super::resolve_builtin_input_schema_ref;

    #[test]
    fn json_schema_discloses_exact_operation_inputs() {
        let schema = resolve_builtin_input_schema_ref("schemas/builtin/json.input.v1.json")
            .expect("JSON schema is registered");
        let branches = schema["oneOf"].as_array().expect("operation alternatives");
        assert_eq!(branches.len(), 13);
        assert_eq!(
            schema["properties"]["operation"]["enum"],
            serde_json::json!([
                "parse",
                "stringify",
                "query",
                "validate",
                "length",
                "last",
                "slice",
                "aggregate"
            ])
        );
        assert!(branches.iter().any(|branch| {
            branch["properties"]["operation"]["const"] == "query"
                && branch["required"] == serde_json::json!(["operation", "data", "path"])
                && branch["not"]["required"] == serde_json::json!(["file_path"])
        }));
        assert!(branches.iter().any(|branch| {
            branch["properties"]["operation"]["const"] == "query"
                && branch["required"] == serde_json::json!(["operation", "file_path", "path"])
                && branch["not"]["required"] == serde_json::json!(["data"])
        }));
        assert_eq!(schema["properties"]["file_path"]["maxLength"], 4096);
        assert!(
            schema["properties"]["file_path"]["description"]
                .as_str()
                .is_some_and(|description| description.contains("8388608 bytes"))
        );
        assert_eq!(schema["properties"]["path"]["maxLength"], 4096);
        assert!(
            schema["properties"]["path"]["description"]
                .as_str()
                .is_some_and(|description| description.contains("$.nodes"))
        );
        assert_eq!(schema["properties"]["start"]["minimum"], 0);
        assert_eq!(schema["properties"]["end"]["minimum"], 0);
        assert_eq!(schema["properties"]["value_index"]["minimum"], 0);
        assert_eq!(
            schema["properties"]["function"]["enum"],
            serde_json::json!(["sum", "average", "min", "max"])
        );
        assert!(branches.iter().any(|branch| {
            branch["properties"]["operation"]["const"] == "slice"
                && branch["required"]
                    == serde_json::json!(["operation", "file_path", "path", "start", "end"])
        }));
        assert!(branches.iter().any(|branch| {
            branch["properties"]["operation"]["const"] == "aggregate"
                && branch["required"]
                    == serde_json::json!(["operation", "data", "path", "function"])
        }));
    }

    #[test]
    fn trigger_create_prompt_description_warns_against_self_referential_creation_prompts() {
        // Issue #5505 (generation-time defense): the model must write the
        // trigger's per-fire action steps, not meta-instructions describing
        // creating/scheduling the trigger itself — otherwise a fired trigger
        // re-invokes trigger_create instead of doing the task ("a routine
        // that creates routines").
        let schema =
            resolve_builtin_input_schema_ref("schemas/builtin/trigger_create.input.v1.json")
                .expect("trigger_create schema is registered");
        let description = schema["properties"]["prompt"]["description"]
            .as_str()
            .expect("prompt description is a string");

        assert!(
            description
                .contains("Do not describe creating, scheduling, or configuring the trigger"),
            "prompt description must warn against self-referential creation prompts: {description}"
        );
    }

    #[test]
    fn trigger_create_prompt_description_scopes_web_app_no_delivery_to_unnamed_destinations() {
        // The categorical "never call builtin__outbound_deliver" web-app
        // clause was observed live being over-applied when the user DID name
        // a destination ("send me a Slack message"): creation turns reasoned
        // "web app → never outbound_deliver" and wrote act-as-user vendor
        // send_message steps to reach the requester instead. The no-delivery
        // default must be scoped to unnamed destinations, and the
        // named-destination case must steer to the pinned
        // builtin__outbound_deliver step, mirroring TRIGGER_CREATE_DESCRIPTION.
        let schema =
            resolve_builtin_input_schema_ref("schemas/builtin/trigger_create.input.v1.json")
                .expect("trigger_create schema is registered");
        let description = schema["properties"]["prompt"]["description"]
            .as_str()
            .expect("prompt description is a string");

        assert!(
            description.contains("no external destination named"),
            "the web-app no-delivery default must be scoped to unnamed destinations: {description}"
        );
        assert!(
            !description.contains("never call builtin__outbound_deliver"),
            "the categorical never-clause invites vendor-send improvisation when a destination IS named: {description}"
        );
        assert!(
            description.contains("names an external destination"),
            "the named-destination case must be explicit, web app included: {description}"
        );
        assert!(
            description.contains("never through integration messaging tools"),
            "messages to the requester must be steered away from act-as-user vendor sends: {description}"
        );
    }

    #[test]
    fn operator_config_schemas_are_registered() {
        let input = resolve_builtin_input_schema_ref(
            "schemas/builtin/operator_config_set_auto_approve.input.v1.json",
        )
        .expect("operator config auto-approve input schema is registered");
        let output = resolve_builtin_input_schema_ref(
            "schemas/builtin/operator_config_set_auto_approve.output.v1.json",
        )
        .expect("operator config auto-approve output schema is registered");

        assert_eq!(input["required"], serde_json::json!(["enabled"]));
        assert_eq!(
            output["properties"]["key"]["const"],
            "agent.auto_approve_tools"
        );
        let tool_input = resolve_builtin_input_schema_ref(
            "schemas/builtin/operator_config_set_tool_permission.input.v1.json",
        )
        .expect("operator config tool-permission input schema is registered");
        let tool_output = resolve_builtin_input_schema_ref(
            "schemas/builtin/operator_config_set_tool_permission.output.v1.json",
        )
        .expect("operator config tool-permission output schema is registered");

        assert_eq!(
            tool_input["properties"]["state"]["enum"],
            serde_json::json!(["default", "always_allow", "ask_each_time", "disabled"])
        );
        assert_eq!(
            tool_output["required"],
            serde_json::json!(["key", "capability_id", "state", "tenant_id", "user_id"])
        );
    }

    /// The schema's advisory `content.maxLength` and the port's authoritative
    /// byte cap (`ironclaw_outbound::MODEL_DELIVERY_MAX_CONTENT_BYTES`) must
    /// carry the same number: the schema is what the model sees, the port is
    /// what enforces, and silent drift between them turns a schema-valid call
    /// into an unexplained rejection.
    #[test]
    fn outbound_deliver_schema_content_bound_matches_the_port_byte_cap() {
        let input =
            resolve_builtin_input_schema_ref("schemas/builtin/outbound_deliver.input.v1.json")
                .expect("outbound_deliver input schema is registered");
        assert_eq!(
            input["properties"]["content"]["maxLength"],
            serde_json::json!(ironclaw_outbound::MODEL_DELIVERY_MAX_CONTENT_BYTES),
        );
    }

    #[test]
    fn ironhub_schemas_require_explicit_catalog_completeness_metadata() {
        let input =
            resolve_builtin_input_schema_ref("schemas/builtin/ironhub_install.input.v1.json")
                .expect("IronHub install input schema is registered");
        let install_output =
            resolve_builtin_input_schema_ref("schemas/builtin/ironhub_install.output.v1.json")
                .expect("IronHub install output schema is registered");
        let search_output =
            resolve_builtin_input_schema_ref("schemas/builtin/ironhub_search.output.v1.json")
                .expect("IronHub search output schema is registered");

        assert!(input["properties"].get("acknowledge_unverified").is_none());
        assert!(
            input["properties"].get("private_manifest_url").is_none(),
            "model-visible IronHub install must not choose a private manifest source"
        );
        assert_eq!(input["additionalProperties"], false);
        assert_eq!(
            install_output["properties"]["phase"]["enum"],
            serde_json::json!(["discovered", "installed"])
        );
        assert_eq!(
            search_output["required"],
            serde_json::json!([
                "phase",
                "total_entries",
                "returned_entries",
                "catalog_total",
                "truncated",
                "entries"
            ])
        );
        assert_eq!(
            search_output["properties"]["total_entries"]["type"],
            "integer"
        );
        assert_eq!(
            search_output["properties"]["returned_entries"]["type"],
            "integer"
        );
        assert_eq!(
            search_output["properties"]["catalog_total"]["type"],
            "integer"
        );
        assert_eq!(search_output["properties"]["truncated"]["type"], "boolean");
        assert_eq!(search_output["additionalProperties"], false);
    }

    #[test]
    fn hosted_mcp_registration_schema_requires_explicit_non_auto_auth() {
        let input = resolve_builtin_input_schema_ref(
            "schemas/builtin/extension_register_hosted_mcp.input.v1.json",
        )
        .expect("hosted MCP registration input schema is registered");

        assert_eq!(
            input["required"],
            serde_json::json!(["desired_id", "desired_name", "endpoint", "auth_type"])
        );
        assert_eq!(
            input["properties"]["auth_type"]["enum"],
            serde_json::json!(["no_auth", "bearer", "oauth"])
        );
        assert_eq!(input["additionalProperties"], false);
    }

    #[test]
    fn reply_attachment_output_schema_exposes_only_opaque_identity_and_safe_metadata() {
        let output = resolve_builtin_input_schema_ref(
            "schemas/builtin/attach_workspace_file_to_reply.output.v1.json",
        )
        .expect("reply attachment output schema is registered");

        assert!(output["properties"].get("path").is_none());
        assert_eq!(
            output["properties"]["attachment_ref"]["pattern"],
            "^att_[0-9a-f]{64}$"
        );
        assert_eq!(
            output["required"],
            serde_json::json!([
                "attached",
                "attachment_ref",
                "filename",
                "mime_type",
                "size_bytes"
            ])
        );
        assert_eq!(output["additionalProperties"], false);
    }

    #[test]
    fn skill_management_mutation_schemas_are_registered() {
        for reference in [
            "schemas/builtin/skill_update.input.v1.json",
            "schemas/builtin/skill_update.output.v1.json",
            "schemas/builtin/skill_auto_activate_set.input.v1.json",
            "schemas/builtin/skill_auto_activate_set.output.v1.json",
            "schemas/builtin/skill_auto_activate_learned_set.input.v1.json",
            "schemas/builtin/skill_auto_activate_learned_set.output.v1.json",
        ] {
            assert!(
                resolve_builtin_input_schema_ref(reference).is_some(),
                "{reference} should be registered"
            );
        }
    }
}
