"""Google Calendar full-path provider operation cases."""

import json

from emulate_provider import google_json
from provider_operation_types import ProviderOperationCase

CALENDAR_ID = "primary"
EVENT_ID = "evt_reborn_planning_sync"
CREATED_EVENT = "REBORN_PROVIDER_CASE_CREATED_EVENT"
UPDATED_SUMMARY = "REBORN_PROVIDER_CASE_UPDATED_EVENT"
ADDED_ATTENDEE = "provider-case-attendee@example.com"


async def _events(emulate_url: str, query: str | None = None) -> list[dict]:
    params: dict[str, str | int] = {"maxResults": 100}
    if query is not None:
        params["q"] = query
    result = await google_json(
        emulate_url,
        "GET",
        f"/calendar/v3/calendars/{CALENDAR_ID}/events",
        params=params,
    )
    assert isinstance(result, dict)
    return result.get("items", [])


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


async def _create_event_baseline(emulate_url: str) -> None:
    assert not await _events(emulate_url, CREATED_EVENT)


async def _create_event_outcome(emulate_url: str, preview: dict) -> None:
    matches = await _events(emulate_url, CREATED_EVENT)
    assert len(matches) == 1, matches
    event = matches[0]
    assert event["summary"] == CREATED_EVENT, event
    assert event["description"] == "Created by the provider operation runner.", event
    assert event["start"]["dateTime"] == "2026-07-30T09:00:00.000Z", event
    assert event["end"]["dateTime"] == "2026-07-30T09:30:00.000Z", event
    assert [attendee["email"] for attendee in event["attendees"]] == [
        "teammate@example.com"
    ], event
    assert CREATED_EVENT in json.dumps(preview), preview


async def _delete_event_baseline(emulate_url: str) -> None:
    matching_ids = {event["id"] for event in await _events(emulate_url)}
    assert EVENT_ID in matching_ids, matching_ids


async def _delete_event_outcome(emulate_url: str, preview: dict) -> None:
    matching_ids = {event["id"] for event in await _events(emulate_url)}
    assert EVENT_ID not in matching_ids, matching_ids
    assert EVENT_ID in json.dumps(preview), preview


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


async def _calendar_list_baseline(emulate_url: str) -> None:
    result = await google_json(
        emulate_url, "GET", "/calendar/v3/users/me/calendarList"
    )
    assert isinstance(result, dict)
    items = result.get("items", [])
    assert any(item.get("id") == CALENDAR_ID for item in items), items


async def _list_calendars_outcome(emulate_url: str, preview: dict) -> None:
    await _calendar_list_baseline(emulate_url)
    assert CALENDAR_ID in json.dumps(preview), preview


GOOGLE_CALENDAR_PROVIDER_OPERATION_CASES = (
    # Executable evidence for a read capability whose harvested journey was
    # quarantined with the retired activation flow (#6520).
    ProviderOperationCase(
        case_id="google_calendar_list_calendars",
        provider_service="google",
        capability_id="google-calendar.list_calendars",
        arguments={},
        assert_baseline=_calendar_list_baseline,
        assert_outcome=_list_calendars_outcome,
    ),
    ProviderOperationCase(
        case_id="google_calendar_create_event",
        provider_service="google",
        capability_id="google-calendar.create_event",
        arguments={
            "calendar_id": CALENDAR_ID,
            "event": {
                "summary": CREATED_EVENT,
                "description": "Created by the provider operation runner.",
                "start": {
                    "dateTime": "2026-07-30T09:00:00.000Z",
                    "timeZone": "UTC",
                },
                "end": {
                    "dateTime": "2026-07-30T09:30:00.000Z",
                    "timeZone": "UTC",
                },
                "attendees": [{"email": "teammate@example.com"}],
            },
        },
        assert_baseline=_create_event_baseline,
        assert_outcome=_create_event_outcome,
    ),
    ProviderOperationCase(
        case_id="google_calendar_delete_event",
        provider_service="google",
        capability_id="google-calendar.delete_event",
        arguments={"calendar_id": CALENDAR_ID, "event_id": EVENT_ID},
        assert_baseline=_delete_event_baseline,
        assert_outcome=_delete_event_outcome,
    ),
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
