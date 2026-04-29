"""Guided replay: same conversation sequence as pipeline_replay but with an explicit
prompt after initiate — naming the exact action — to test whether that prevents the
initiate→execute skip seen in the 23:10:24.784 log."""

import json

from common import RESP_PAT, agent_text, get_tool_calls, format_tool_calls, fetch_account_info, tool_output_ok

MAX_TURNS = 20


def run_pipeline_replay_guided(
    run_id: int,
    client,
    read_token: str,
    api_key: str,
    temperature: float,
    runs_per_thread: int,
) -> tuple[list[tuple[str, bool, str]], list[str]]:
    checks: list[tuple[str, bool, str]] = []
    log: list[str] = []
    tag = f"replay_guided-{run_id}"

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

    if not recipients or not funding_sources:
        chk("account has data", False, 0, f"recipients={len(recipients)} funding_sources={len(funding_sources)}")
        return checks, log
    chk("account has data", True, 0)

    recipient = recipients[0]
    funding_source = funding_sources[0]
    ir015 = next((r for r in payment_reasons if r["key"] == "IR015"), None)
    if not ir015:
        chk("IR015 payment reason available", False, 0, "IR015 not found in account payment reasons")
        return checks, log
    chk("IR015 payment reason available", True, 0)

    note(f"  recipient:      {recipient['name']} (ending in {recipient['mask'].lstrip('*')})", 0)
    note(f"  funding source: {funding_source['bank_name']} ending in {funding_source['mask']}", 0)
    note(f"  payment reason: {ir015['value']} ({ir015['key']})", 0)

    # Fixed replay script — matches the observed conversation from the log.
    # Slots are filled with real account values from the test user.
    def script(sub: int) -> list[str]:
        mask = recipient["mask"].lstrip("*")
        return [
            "Send money to indi",
            f"Send money to {recipient['name']} (account ending in {mask})",
            f"Use {funding_source['bank_name']} account ending in {funding_source['mask']}",
            "0.7$",
            "Let's do 15",
            "The payment reason is Investment",
            'send now using abound_send_wire("send")',
        ]

    prev_id = None
    thread_uuid_first = None

    for sub in range(1, runs_per_thread + 1):
        note(f"=== Sub-run {sub}/{runs_per_thread} ===", sub)

        turns = script(sub)
        turn_idx = 0

        initiate_seen = False
        initiate_call_args = {}
        send_response = None
        send_call_args = {}
        send_output = None
        send_agent_text = None
        execute_seen = False

        for turn in range(1, MAX_TURNS + 1):
            if turn_idx < len(turns):
                next_input = turns[turn_idx]
                turn_idx += 1
            elif initiate_seen and not send_response:
                next_input = 'send now using abound_send_wire("send")'
            else:
                next_input = "Please proceed."

            note(f"--- Turn {turn} ---", sub)
            note(f"  input: {next_input}", sub)

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
                if "abound_send_wire" not in c["name"]:
                    continue
                action = c["args"].get("action") or ""
                if not action:
                    _, _, suffix = c["name"].partition("(")
                    if suffix.endswith(")"):
                        action = suffix[:-1]
                output = c.get("output") or ""

                # Detect initiate success from output JSON — works even when
                # FunctionCall.arguments is empty (Responses API bug on success).
                if not initiate_seen and output and not output.startswith("Error:"):
                    try:
                        result = json.loads(output)
                        if result.get("phase") == "confirmation_required":
                            initiate_seen = True
                            initiate_call_args = c["args"]
                            note("  initiate: confirmation_required", sub)
                    except Exception:
                        pass

                # Log initiate failures clearly instead of falling back to markdown.
                if action == "initiate":
                    if not output:
                        note("  initiate: output not captured", sub)
                    elif output.startswith("Error:"):
                        note(f"  initiate error: {output[:200]}", sub)

                if action == "send":
                    send_response = resp
                    send_call_args = c["args"]
                    send_output = c.get("output")
                    send_agent_text = text
                if action == "execute":
                    execute_seen = True
                    note(f"  execute called (skipped send!): {json.dumps(c['args'])}", sub)

            if text:
                note(f"  agent text: {text[:300]}", sub)

            chk(f"turn {turn} completed", resp.status == "completed", sub, f"status={resp.status}")
            prev_id = resp.id

            if send_response or execute_seen:
                note("", sub)
                break

        note("--- Assertions ---", sub)
        chk("action=initiate was called", initiate_seen, sub)
        chk("action=send was called (not skipped)", send_response is not None, sub,
            "model called execute directly, skipping send" if execute_seen else "model did not call send")
        if send_response is not None:
            chk("action=send tool executed", tool_output_ok(send_output, send_agent_text), sub,
                f"no output or tool error: {send_output!r}")
        chk("action=execute was NOT called directly", not execute_seen, sub,
            f"execute fired without send: {json.dumps(send_call_args)}")

        if send_response and initiate_call_args:
            expected = {**initiate_call_args, "notify_thread_id": thread_uuid_first}
            note(f"  expected: {json.dumps(expected)}", sub)
            note(f"  actual:   {json.dumps(send_call_args)}", sub)
            for key, expected_val in sorted(expected.items()):
                actual_val = send_call_args.get(key)
                chk(f"send.{key}", actual_val == expected_val, sub,
                    f"got {actual_val!r}, expected {expected_val!r}")

    return checks, log
