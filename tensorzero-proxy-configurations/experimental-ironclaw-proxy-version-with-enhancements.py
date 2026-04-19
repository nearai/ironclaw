#!/usr/bin/env python3
"""
IronClaw → TensorZero Proxy with /me Support

Sits between IronClaw and TensorZero, cleaning responses to be
strictly OpenAI-compatible by removing TensorZero-specific fields
that IronClaw's Rust parser can't handle (episode_id, tensorzero_cost, etc.).

NEW FEATURES (ported from OpenClaw proxy):
- Detects /me commands and routes to slash_me function
- Handles both literal "/me" and IRC ACTION format
- Strips WeeChat metadata preamble from all messages
- Normal messages route to default function

Usage:
    python3 ironclaw-proxy-with-me.py --port 3002 --tensorzero http://192.168.1.157:3000
"""

import re
import json
import socketserver
import http.server
import urllib.request
import urllib.error
import argparse
import sys
import traceback

TENSORZERO_URL = "http://192.168.1.157:3000"
PROXY_PORT = 3002
CTCP_CHAR = '\x01'

METADATA_PATTERN = re.compile(
    r'(?:Conversation info|Sender)\s*\(untrusted metadata\):\s*```json\s*\{[^}]*\}\s*```\s*',
    re.DOTALL
)

def route_function(message: str, default_function: str = "openclaw") -> str:
    if re.search(r'(?:^|\s)/me\b', message):
        return "slash_me"
    if f'{CTCP_CHAR}ACTION ' in message:
        return "slash_me"
    return default_function

def clean_message(msg: str) -> str:
    match = re.search(rf'{CTCP_CHAR}ACTION\s+(.*?){CTCP_CHAR}', msg)
    if match:
        return match.group(1).strip()
    cleaned = re.sub(r'(?:^|\n)\s*/me\s+', '', msg).strip()
    return cleaned if cleaned != msg.strip() else msg.strip()

def strip_metadata(msg: str) -> str:
    if not isinstance(msg, str):
        return msg
    stripped = METADATA_PATTERN.sub('', msg).strip()
    return stripped if stripped else msg

def strip_metadata_from_content(content):
    if isinstance(content, str):
        return strip_metadata(content)
    elif isinstance(content, list):
        for i, part in enumerate(content):
            if isinstance(part, dict) and part.get('type') == 'text':
                part['text'] = strip_metadata(part.get('text', ''))
            elif isinstance(part, str):
                content[i] = strip_metadata(part)
        return content
    return content

def sanitize_roles_for_roleplay(messages: list) -> list:
    VALID_ROLES = {"user", "system", "assistant"}
    ROLE_MAP = {"developer": "system"}
    DROP_ROLES = {"tool", "function"}
    sanitized = []
    for msg in messages:
        role = msg.get("role", "")
        if role in DROP_ROLES:
            continue
        if role in VALID_ROLES:
            cleaned = msg
        else:
            cleaned = msg.copy()
            cleaned["role"] = ROLE_MAP.get(role, "user")
        if cleaned.get("tool_calls"):
            cleaned = cleaned.copy()
            cleaned.pop("tool_calls", None)
        if "tool_call_id" in cleaned:
            cleaned = cleaned.copy()
            cleaned.pop("tool_call_id", None)
        sanitized.append(cleaned)
    return sanitized

def clean_response(data: dict) -> dict:
    cleaned = {
        "id": data.get("id", ""),
        "object": data.get("object", "chat.completion"),
        "created": data.get("created", 0),
        "model": data.get("model", ""),
        "choices": [],
        "usage": {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0},
    }
    usage = data.get("usage", {})
    if isinstance(usage, dict):
        cleaned["usage"] = {
            "prompt_tokens": usage.get("prompt_tokens", 0),
            "completion_tokens": usage.get("completion_tokens", 0),
            "total_tokens": usage.get("total_tokens", 0),
        }
    for choice in data.get("choices", []):
        clean_choice = {
            "index": choice.get("index", 0),
            "finish_reason": choice.get("finish_reason", "stop"),
        }
        msg = choice.get("message", {})
        content = msg.get("content") or ""
        if not content.strip():
            content = msg.get("reasoning_content") or msg.get("reasoning") or ""
            if content:
                content = re.sub(r'<think>\s*', '', content)
                content = re.sub(r'\s*