"""Specific pipeline: answers each agent question with real Abound account data.

Unlike choice_set (which always picks the first option), this pipeline fetches
real account info and selects the matching item from each choice_set. Starts
with a generic opening message and responds one question at a time.
"""

import json

from common import RESP_PAT, CHOICE_PAT, agent_text, get_tool_calls, format_tool_calls, fetch_account_info, tool_output_ok

MAX_TURNS = 12


def _pick_from_account(text: str, account: dict) -> str | None:
    """Find and return the prompt for the choice_set item that matches our account data."""
    recipient = account["recipients"][0]
    funding_source = account["funding_sources"][0]
    payment_reason = account["payment_reasons"][0]

    for m in CHOICE_PAT.finditer(text):
        try:
            data = json.loads(m.group(1).strip())
            choice_id = data.get("id", "").lower()
            items = data.get("items", [])
            if not items:
                continue

            if "recipient" in choice_id:
                mask = recipient["mask"].lstrip("*")
                for item in items:
                    if mask in item.get("prompt", ""):
                        return item["prompt"]

            elif "payment" in choice_id or "reason" in choice_id:
                target = payment_reason["value"]
                for item in items:
                    prompt = item.get("prompt", "")
                    if target in prompt or payment_reason["key"] in item.get("value", ""):
                        return prompt

            elif "funding" in choice_id or "source" in choice_id or "account" in choice_id:
                mask = funding_source["mask"]
                for item in items:
                    if mask in item.get("prompt", ""):
                        return item["prompt"]

            # Unknown choice_set type — fall back to first item
            return items[0].get("prompt")

        except Exception:
            pass

    return None


def run_pipeline_specific(
    run_id: int,
    client,
    read_token: str,
    api_key: str,
    temperature: float,
    runs_per_thread: int,
) -> tuple[list[tuple[str, bool, str]], list[str]]:
    """Run runs_per_thread sequential sub-runs, answering agent questions with real account data."""
    checks: list[tuple[str, bool, str]] = []
    log: list[str] = []
    tag = f"specific-{run_id}"

    def note(msg: str, sub: int) -> None:
        log.append(f"[{tag}/sub-{sub}] {msg}")

    def chk(name: str, condition: bool, sub: int, detail: str = "") -> bool:
        checks.append((f"sub-{sub} {name}", condition, detail))
        status = "PASS" if condition else "FAIL"
        suffix = f"\n[{tag}/sub-{sub}]    {detail[:300]}" if detail and not condition else ""
        note(f"  {status}: {name}{suffix}", sub)
        return condition

    note("--- Fetching account info ---", 0)
    try:
        account = fetch_account_info(read_token, api_key)
    except Exception as e:
        chk("fetch account info", False, 0, str(e))
        return checks, log
    chk("fetch account info", True, 0)

    recipients = account.get("recipients", [])
    funding_sources = account.get("funding_sources", [])
    payment_reasons = account.get("payment_reasons", [])

    if not recipients or not funding_sources or not payment_reasons:
        chk("account has data", False, 0,
            f"recipients={len(recipients)} funding_sources={len(funding_sources)} payment_reasons={len(payment_reasons)}")
        return checks, log
    chk("account has data", True, 0)

    note(f"  recipient: {recipients[0]['name']} ({recipients[0]['mask']})", 0)
    note(f"  funding:   {funding_sources[0]['bank_name']} ending in {funding_sources[0]['mask']}", 0)
    note(f"  reason:    {payment_reasons[0]['value']}", 0)

    prev_id = None
    thread_uuid_first = None

    for sub in range(1, runs_per_thread + 1):
        note(f"=== Sub-run {sub}/{runs_per_thread} ===", sub)

        first_input = "I want to send $15 to India." if sub == 1 else "Now I want to send another $15 to India."

        initiate_seen = False
        initiate_call_args = {}
        send_response = None
        send_call_args = {}
        send_output = None
        next_input = first_input

        for turn in range(1, MAX_TURNS + 1):
            note(f"--- Turn {turn} {'(initiate seen)' if initiate_seen else ''} ---", sub)
            note(f"  input: {next_input[:200]}", sub)

            kwargs = {"model": "default", "input": next_input, "timeout": 180}
            if temperature is not None:
                kwargs["temperature"] = temperature
            if prev_id:
                kwargs["previous_response_id"] = prev_id

            try:
                resp = client.responses.create(**kwargs)
            except Exception as e:
                chk(f"turn {turn} request", False, sub, str(e))
                return checks, log

            m = RESP_PAT.match(resp.id)
            thread_uuid = m.group(2) if m else None
            if thread_uuid and thread_uuid_first is None:
                thread_uuid_first = thread_uuid
            note(f"  response.id: {resp.id}", sub)

            calls = get_tool_calls(resp)
            text = agent_text(resp)
            for line in format_tool_calls(calls, f"[{tag}/sub-{sub}]"):
                log.append(line)

            for c in calls:
                if "abound_send_wire" in c["name"]:
                    action = c["args"].get("action") or c["name"].split("(")[-1].rstrip(")")
                    if action == "initiate":
                        try:
                            initiate_result = json.loads(c.get("output") or text)
                            if initiate_result.get("phase") == "confirmation_required":
                                initiate_seen = True
                                initiate_call_args = c["args"]
                                note("  initiate: confirmation_required", sub)
                            else:
                                note(f"  initiate rejected: {text[:200]}", sub)
                        except Exception:
                            note(f"  initiate parse failed: {text[:200]}", sub)
                    if action == "send":
                        send_response = resp
                        send_call_args = c["args"]
                        send_output = c.get("output")

            if text and not any("abound_send_wire" in c["name"] and
                                (c["args"].get("action") or "") == "initiate"
                                for c in calls):
                note(f"  agent text: {text[:300]}", sub)

            chk(f"turn {turn} completed", resp.status == "completed", sub, f"status={resp.status}")
            prev_id = resp.id

            if send_response:
                note("", sub)
                break

            if initiate_seen:
                next_input = "Send now."
            else:
                choice = _pick_from_account(text, account)
                if choice:
                    note(f"  account-pick: {choice[:120]}", sub)
                    next_input = choice
                else:
                    next_input = "Please proceed."

            note("", sub)

        note("--- Assertions ---", sub)
        chk("action=initiate was called", initiate_seen, sub)
        chk("action=send was called", send_response is not None, sub,
            "model did not call send after 'Send now.'")
        if send_response is not None:
            chk("action=send tool executed", tool_output_ok(send_output), sub,
                f"no output or tool error: {send_output!r}")

        if send_response and initiate_call_args:
            expected = {**initiate_call_args, "notify_thread_id": thread_uuid_first}
            note(f"  expected: {json.dumps(expected)}", sub)
            note(f"  actual:   {json.dumps(send_call_args)}", sub)
            for key, expected_val in sorted(expected.items()):
                actual_val = send_call_args.get(key)
                chk(f"send.{key}", actual_val == expected_val, sub,
                    f"got {actual_val!r}, expected {expected_val!r}")

    return checks, log
