"""Choice-set pipeline: auto-picks from [[choice_set]] menus until action=send fires."""

import json

from common import RESP_PAT, CHOICE_PAT, agent_text, get_tool_calls, format_tool_calls, first_choice_prompt, tool_output_ok

MAX_TURNS = 12


def run_pipeline(
    run_id: int,
    client,
    temperature: float,
    runs_per_thread: int,
) -> tuple[list[tuple[str, bool, str]], list[str]]:
    """Run runs_per_thread sequential sub-runs in one conversation thread."""
    checks: list[tuple[str, bool, str]] = []
    log: list[str] = []
    tag = f"run-{run_id}"

    def note(msg: str, sub: int) -> None:
        log.append(f"[{tag}/sub-{sub}] {msg}")

    def chk(name: str, condition: bool, sub: int, detail: str = "") -> bool:
        checks.append((f"sub-{sub} {name}", condition, detail))
        status = "PASS" if condition else "FAIL"
        suffix = f"\n[{tag}/sub-{sub}]    {detail[:300]}" if detail and not condition else ""
        note(f"  {status}: {name}{suffix}", sub)
        return condition

    prev_id = None
    thread_uuid_first = None

    for sub in range(1, runs_per_thread + 1):
        first_input = "I want to send $50 to India." if sub == 1 else "Now I want to send another $123 to India."
        note(f"=== Sub-run {sub}/{runs_per_thread} ===", sub)

        initiate_seen = False
        initiate_call_args = {}
        send_response = None
        send_call_args = {}
        send_output = None
        send_agent_text = None
        next_input = first_input

        for turn in range(1, MAX_TURNS + 1):
            note(f"--- Turn {turn} {'(initiate seen)' if initiate_seen else ''} ---", sub)
            note(f"  input: {next_input[:120]}", sub)

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
                        send_agent_text = text

            if text and not any("abound_send_wire" in c["name"] and
                                (c["args"].get("action") or "") == "initiate"
                                for c in calls):
                note(f"  agent text: {text[:300]}", sub)

            chk(f"turn {turn} completed", resp.status == "completed", sub, f"status={resp.status}")
            prev_id = resp.id

            if send_response:
                note("", sub)
                break

            choice = first_choice_prompt(text)
            if choice:
                note(f"  auto-pick: {choice[:120]}", sub)
                next_input = choice
            elif initiate_seen:
                next_input = "Send now."
            else:
                next_input = "Please proceed."
            note("", sub)

        note("--- Assertions ---", sub)
        chk("action=initiate was called", initiate_seen, sub)
        chk("action=send was called", send_response is not None, sub,
            "model did not call send after 'Send now.'")
        if send_response is not None:
            chk("action=send tool executed", tool_output_ok(send_output, send_agent_text), sub,
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
