"""Google Calendar full-path provider operation cases."""

import json

from provider_operation_google_common import google_json
from provider_operation_types import ProviderOperationCase

CALENDAR_ID = "primary"
EVENT_ID = "evt_reborn_planning_sync"
UPDATED_SUMMARY = "REBORN_PROVIDER_CASE_UPDATED_EVENT"
ADDED_ATTENDEE = "provider-case-attendee@example.com"


async def _event(emulate_url: str) -> dict:
    result = await google_json(
        emulate_url,
        "GET",
        f"/calendar/v3/calendars/{CALENDAR_ID}/events/{EVENT_ID}",
    )
    assert isinstance(result, dict)
    return result


async def _seeded_event_baseline(emulate_url: str) -> None:
    event = await _event(emulate_url)
    assert event["summary"] == "Reborn planning sync", event


async def _get_event_outcome(emulate_url: str, preview: dict) -> None:
    await _seeded_event_baseline(emulate_url)
    assert "Reborn planning sync" in json.dumps(preview), preview


async def _free_busy_baseline(emulate_url: str) -> None:
    await _seeded_event_baseline(emulate_url)


async def _free_busy_outcome(emulate_url: str, preview: dict) -> None:
    await _seeded_event_baseline(emulate_url)
    rendered = json.dumps(preview)
    assert EVENT_ID not in rendered
    assert "2026-06-22T13:00:00" in rendered, preview
    assert "2026-06-22T13:30:00" in rendered, preview


async def _update_event_outcome(emulate_url: str, preview: dict) -> None:
    event = await _event(emulate_url)
    assert event["summary"] == UPDATED_SUMMARY, event
    assert UPDATED_SUMMARY in json.dumps(preview), preview


async def _add_attendees_outcome(emulate_url: str, preview: dict) -> None:
    event = await _event(emulate_url)
    emails = [attendee["email"] for attendee in event["attendees"]]
    assert emails == ["teammate@example.com", ADDED_ATTENDEE], event
    assert ADDED_ATTENDEE in json.dumps(preview), preview


async def _set_reminder_outcome(emulate_url: str, preview: dict) -> None:
    event = await _event(emulate_url)
    assert event["reminders"] == {
        "useDefault": False,
        "overrides": [{"method": "popup", "minutes": 15}],
    }, event
    assert EVENT_ID in json.dumps(preview), preview


GOOGLE_CALENDAR_PROVIDER_OPERATION_CASES = (
    ProviderOperationCase(
        case_id="google_calendar_get_event",
        provider_service="google",
        capability_id="google-calendar.get_event",
        arguments={"calendar_id": CALENDAR_ID, "event_id": EVENT_ID},
        assert_baseline=_seeded_event_baseline,
        assert_outcome=_get_event_outcome,
    ),
    ProviderOperationCase(
        case_id="google_calendar_find_free_slots",
        provider_service="google",
        capability_id="google-calendar.find_free_slots",
        arguments={
            "timeMin": "2026-06-22T12:45:00.000Z",
            "timeMax": "2026-06-22T13:45:00.000Z",
            "timeZone": "UTC",
            "items": [{"id": CALENDAR_ID}],
        },
        assert_baseline=_free_busy_baseline,
        assert_outcome=_free_busy_outcome,
    ),
    ProviderOperationCase(
        case_id="google_calendar_update_event",
        provider_service="google",
        capability_id="google-calendar.update_event",
        arguments={
            "calendar_id": CALENDAR_ID,
            "event_id": EVENT_ID,
            "event": {"summary": UPDATED_SUMMARY},
        },
        assert_baseline=_seeded_event_baseline,
        assert_outcome=_update_event_outcome,
    ),
    ProviderOperationCase(
        case_id="google_calendar_add_attendees",
        provider_service="google",
        capability_id="google-calendar.add_attendees",
        arguments={
            "calendar_id": CALENDAR_ID,
            "event_id": EVENT_ID,
            "attendees": [{"email": ADDED_ATTENDEE}],
        },
        assert_baseline=_seeded_event_baseline,
        assert_outcome=_add_attendees_outcome,
    ),
    ProviderOperationCase(
        case_id="google_calendar_set_reminder",
        provider_service="google",
        capability_id="google-calendar.set_reminder",
        arguments={
            "calendar_id": CALENDAR_ID,
            "event_id": EVENT_ID,
            "reminders": {
                "useDefault": False,
                "overrides": [{"method": "popup", "minutes": 15}],
            },
        },
        assert_baseline=_seeded_event_baseline,
        assert_outcome=_set_reminder_outcome,
    ),
)
