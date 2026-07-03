"""Semantic completion judge for Reborn WebUI v2 live QA text checks."""

from __future__ import annotations

import json
import os

from scripts.live_canary.common import env_secret


async def _judge_assistant_reply_completion(
    *,
    marker: str | None,
    required_text: list[str],
    assistant_text: str,
    main_text: str,
    semantic_goal: str | None,
) -> dict[str, object] | None:
    if not _env_flag_enabled("REBORN_WEBUI_V2_LIVE_QA_LLM_JUDGE", default=True):
        return {"enabled": False, "reason": "disabled"}
    api_key_env = _judge_api_key_env()
    api_key = env_secret(api_key_env)
    if not api_key:
        return {"enabled": False, "reason": f"{api_key_env} unset"}

    try:
        import httpx

        async with httpx.AsyncClient(timeout=30.0) as client:
            response = await client.post(
                f"{_judge_base_url()}/chat/completions",
                headers={
                    "Authorization": f"Bearer {api_key}",
                    "Content-Type": "application/json",
                },
                json=_judge_payload(
                    marker=marker,
                    required_text=required_text,
                    assistant_text=assistant_text,
                    main_text=main_text,
                    semantic_goal=semantic_goal,
                ),
            )
            response.raise_for_status()
            body = response.json()
            content = _completion_content(body)
    except Exception as exc:
        return {"enabled": True, "error": str(exc)}

    parsed = _parse_json_object(content)
    if not isinstance(parsed, dict):
        return {
            "enabled": True,
            "error": "judge response was not a JSON object",
            "response_excerpt": str(content)[-500:],
        }
    parsed["enabled"] = True
    return parsed


def _semantic_judge_passed(result: dict[str, object] | None) -> bool:
    if not result or result.get("completed") is not True:
        return False
    try:
        confidence = float(result.get("confidence") or 0.0)
    except (TypeError, ValueError):
        confidence = 0.0
    try:
        threshold = float(
            os.environ.get("REBORN_WEBUI_V2_LIVE_QA_LLM_JUDGE_MIN_CONFIDENCE", "0.75")
        )
    except ValueError:
        threshold = 0.75
    return confidence >= threshold


def _compact_json(value: object) -> str:
    if value is None:
        return "null"
    try:
        encoded = json.dumps(value, sort_keys=True)
    except TypeError:
        encoded = repr(value)
    if len(encoded) > 1000:
        return f"{encoded[:1000]}..."
    return encoded


def _judge_api_key_env() -> str:
    return os.environ.get(
        "REBORN_WEBUI_V2_LIVE_QA_LLM_JUDGE_API_KEY_ENV",
        os.environ.get(
            "REBORN_WEBUI_V2_LIVE_QA_LLM_API_KEY_ENV",
            "NEARAI_API_KEY"
            if os.environ.get("NEARAI_API_KEY") or os.environ.get("NEARAI_API_KEY_PATH")
            else "LIVE_OPENAI_COMPATIBLE_API_KEY",
        ),
    )


def _judge_base_url() -> str:
    return os.environ.get(
        "REBORN_WEBUI_V2_LIVE_QA_LLM_JUDGE_BASE_URL",
        os.environ.get(
            "REBORN_WEBUI_V2_LIVE_QA_LLM_BASE_URL",
            os.environ.get("LIVE_OPENAI_COMPATIBLE_BASE_URL", "https://cloud-api.near.ai/v1"),
        ),
    ).rstrip("/")


def _judge_model() -> str:
    return os.environ.get(
        "REBORN_WEBUI_V2_LIVE_QA_LLM_JUDGE_MODEL",
        os.environ.get(
            "REBORN_WEBUI_V2_LIVE_QA_LLM_MODEL",
            os.environ.get("LIVE_OPENAI_COMPATIBLE_MODEL", "deepseek-ai/DeepSeek-V4-Flash"),
        ),
    )


def _judge_payload(
    *,
    marker: str | None,
    required_text: list[str],
    assistant_text: str,
    main_text: str,
    semantic_goal: str | None,
) -> dict[str, object]:
    return {
        "model": _judge_model(),
        "temperature": 0,
        "max_tokens": 500,
        "messages": [
            {
                "role": "system",
                "content": (
                    "You are a strict semantic verifier for a live WebUI QA canary. "
                    "Return JSON only. Judge only whether the visible assistant response "
                    "semantically satisfies the natural-language expectation. Do not infer "
                    "external side effects, database writes, tool calls, delivery, auth, or "
                    "capability execution unless the text explicitly says so. If an exact "
                    "marker is required and missing, completed must be false."
                ),
            },
            {
                "role": "user",
                "content": json.dumps(
                    {
                        "task_prompt": semantic_goal or "",
                        "exact_marker_required": marker,
                        "literal_required_text": required_text,
                        "assistant_response": assistant_text[-4000:],
                        "page_excerpt": main_text[-2000:],
                        "required_json_schema": {
                            "completed": "boolean",
                            "confidence": "number from 0 to 1",
                            "reason": "short string",
                            "evidence": "array of short strings",
                            "missing": "array of short strings",
                        },
                    },
                    sort_keys=True,
                ),
            },
        ],
    }


def _completion_content(body: object) -> object:
    if not isinstance(body, dict):
        return ""
    choices = body.get("choices")
    if not isinstance(choices, list) or not choices:
        return ""
    choice = choices[0]
    if not isinstance(choice, dict):
        return ""
    message = choice.get("message")
    if not isinstance(message, dict):
        return ""
    return message.get("content") or ""


def _parse_json_object(text: object) -> object:
    if not isinstance(text, str):
        return None
    try:
        return json.loads(text)
    except Exception:
        start = text.find("{")
        end = text.rfind("}")
        if start == -1 or end <= start:
            return None
        try:
            return json.loads(text[start : end + 1])
        except Exception:
            return None


def _env_flag_enabled(name: str, *, default: bool) -> bool:
    raw = os.environ.get(name)
    if raw is None:
        return default
    return raw.strip().lower() not in {"0", "false", "no", "off"}
