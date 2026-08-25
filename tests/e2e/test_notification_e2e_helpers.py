import asyncio

import httpx

import notification_e2e_helpers as helpers


def response(status_code, *, json=None, headers=None):
    return httpx.Response(
        status_code,
        json=json,
        headers=headers,
        request=httpx.Request("GET", "http://example.test"),
    )


class SequencedClient:
    def __init__(self, *, gets, post=None):
        self.gets = list(gets)
        self.post_response = post
        self.get_count = 0

    async def get(self, *_args, **_kwargs):
        self.get_count += 1
        return self.gets.pop(0)

    async def post(self, *_args, **_kwargs):
        assert self.post_response is not None
        return self.post_response


async def test_create_automation_retries_a_throttled_list(monkeypatch):
    expected_name = helpers.notification_automation_name("approval", "retry")
    client = SequencedClient(
        gets=[
            response(429, headers={"retry-after": "2.5"}),
            response(200, json={"automations": [{"name": expected_name}]}),
        ]
    )
    sleeps = []

    async def no_op(*_args, **_kwargs):
        return None

    async def capture_sleep(delay) -> None:
        sleeps.append(delay)

    monkeypatch.setattr(helpers, "create_thread", no_op)
    monkeypatch.setattr(helpers, "send_message", no_op)
    monkeypatch.setattr(helpers, "wait_for_assistant_message", no_op)
    monkeypatch.setattr(asyncio, "sleep", capture_sleep)

    automation = await helpers.create_notification_automation(
        client, "http://example.test", "approval", "retry"
    )

    assert automation["name"] == expected_name
    assert client.get_count == 2
    assert sleeps == [2.5]


async def test_run_automation_retries_a_throttled_projection(monkeypatch):
    client = SequencedClient(
        post=response(200, json={"run_result": {"run_id": "run-1"}}),
        gets=[
            response(429, headers={"retry-after": "0"}),
            response(
                200,
                json={
                    "automations": [
                        {
                            "automation_id": "automation-1",
                            "recent_runs": [
                                {"run_id": "run-1", "thread_id": "thread-1"}
                            ],
                        }
                    ]
                },
            ),
        ],
    )
    sleeps = []

    async def capture_sleep(delay) -> None:
        sleeps.append(delay)

    monkeypatch.setattr(asyncio, "sleep", capture_sleep)

    run = await helpers.run_notification_automation(
        client, "http://example.test", "automation-1"
    )

    assert run["run_id"] == "run-1"
    assert client.get_count == 2
    assert sleeps == [0.5]


async def test_rate_limit_retry_uses_safe_default_for_invalid_header(monkeypatch):
    sleeps = []

    async def capture_sleep(delay) -> None:
        sleeps.append(delay)

    monkeypatch.setattr(asyncio, "sleep", capture_sleep)
    deadline = asyncio.get_running_loop().time() + 10.0

    assert await helpers.retry_after_rate_limit(
        response(429, headers={"retry-after": "not-a-number"}), deadline=deadline
    )
    assert sleeps == [1.0]


async def test_rate_limit_retry_never_sleeps_past_polling_deadline(monkeypatch):
    sleeps = []

    async def capture_sleep(delay) -> None:
        sleeps.append(delay)

    monkeypatch.setattr(asyncio, "sleep", capture_sleep)
    deadline = asyncio.get_running_loop().time() + 0.25

    assert await helpers.retry_after_rate_limit(
        response(429, headers={"retry-after": "3600"}), deadline=deadline
    )
    assert len(sleeps) == 1
    assert 0.0 <= sleeps[0] <= 0.25


async def test_rate_limit_retry_rejects_non_finite_header(monkeypatch):
    sleeps = []

    async def capture_sleep(delay) -> None:
        sleeps.append(delay)

    monkeypatch.setattr(asyncio, "sleep", capture_sleep)
    deadline = asyncio.get_running_loop().time() + 10.0

    assert await helpers.retry_after_rate_limit(
        response(429, headers={"retry-after": "nan"}), deadline=deadline
    )
    assert sleeps == [1.0]
