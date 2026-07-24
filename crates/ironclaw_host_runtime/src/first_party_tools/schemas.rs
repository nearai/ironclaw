use serde_json::{Value, json};

use crate::first_party_tools::time::UNIX_MILLIS_THRESHOLD;

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
                    "enum": ["parse", "stringify", "query", "validate"]
                },
                "data": { "description": "JSON string or JSON value to process" },
                "path": { "type": "string", "description": "Dot/bracket path for query operation" }
            },
            "required": ["operation", "data"],
            "additionalProperties": false
        }),
        "schemas/builtin/http.input.v1.json" => http_schema(false),
        "schemas/builtin/outbound_delivery_target_route_current.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "target_id": {
                    "type": "string",
                    "description": "Opaque target id returned by builtin__outbound_delivery_targets_list."
                }
            },
            "required": ["target_id"],
            "additionalProperties": false
        }),
        "schemas/builtin/outbound_delivery_target_route_current.output.v1.json" => json!({
            "type": "object",
            "properties": {
                "routed": { "type": "boolean", "const": true }
            },
            "required": ["routed"],
            "additionalProperties": false
        }),
        "schemas/builtin/http-save.input.v1.json" => http_schema(true),
        "schemas/builtin/memory_search.input.v1.json" => json!({
            "type": "object",
            "description": "Searches only Reborn internal persistent memory. This does not search connected app or extension data.",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Preferred natural language search query for Reborn internal persistent memory"
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
        "schemas/builtin/extension_install.input.v1.json"
        | "schemas/builtin/extension_remove.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "extension_id": { "type": "string", "description": "Extension id from extension_search results" }
            },
            "required": ["extension_id"],
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
        "schemas/builtin/outbound_preferences_set.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "final_reply_target_id": {
                    "type": ["string", "null"],
                    "maxLength": 512,
                    "description": "Outbound delivery target id to use for final replies. Omit or pass null to clear the preference."
                }
            },
            "additionalProperties": false
        }),
        "schemas/builtin/outbound_preferences_set.output.v1.json" => json!({
            "type": "object",
            "properties": {
                "final_reply_target": {
                    "type": ["object", "null"]
                },
                "final_reply_target_status": { "type": "string" },
                "default_modality": { "type": "string" }
            },
            "required": ["final_reply_target", "final_reply_target_status", "default_modality"],
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
                    "description": "Prompt submitted when the trigger fires. Write only the action performed at fire time. If delivery_target_id is set, never put a send, post, or deliver-results step for that result here; the host delivers the final reply automatically. Never tell the prompt to send results back to the requesting user — receiving results is routing, never a prompt step, even when phrased as 'send me the result' or when the requester's conversation id is known. Put messaging here only when messaging someone else is itself the task, and pin that third-party recipient while the user is present. Do not describe creating, scheduling, or configuring the trigger. Runtime validation caps UTF-8 content at 32768 bytes."
                },
                "delivery_target_id": {
                    "type": "string",
                    "description": "Optional per-trigger outbound delivery target id from builtin__outbound_delivery_targets_list. When set, the host delivers this trigger's final results to that target. Do not also put a send, post, or deliver-results step for that result in prompt. When omitted, the host inherits the current source run's authorized delivery route when one exists; otherwise the user's default outbound delivery target at fire time is used. This is resolved from trusted run state, never prompt parsing. Prefer setting this whenever the user names a destination for this trigger's results."
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

    #[test]
    fn outbound_preferences_set_schemas_are_registered() {
        let input = resolve_builtin_input_schema_ref(
            "schemas/builtin/outbound_preferences_set.input.v1.json",
        )
        .expect("outbound preferences set input schema is registered");
        let output = resolve_builtin_input_schema_ref(
            "schemas/builtin/outbound_preferences_set.output.v1.json",
        )
        .expect("outbound preferences set output schema is registered");

        assert_eq!(
            input["properties"]["final_reply_target_id"]["type"],
            serde_json::json!(["string", "null"])
        );
        assert_eq!(
            output["required"],
            serde_json::json!([
                "final_reply_target",
                "final_reply_target_status",
                "default_modality"
            ])
        );
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
