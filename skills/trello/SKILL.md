---
name: trello
version: "1.0.0"
description: Trello REST API — boards, lists, cards, comments, labels
activation:
  keywords:
    - "trello"
    - "trello board"
    - "trello card"
  exclude_keywords:
    - "jira"
    - "asana"
  patterns:
    - "(?i)trello.*(board|card|list)"
    - "(?i)(create|move|update).*trello"
  tags:
    - "project-management"
    - "kanban"
  max_context_tokens: 1500
metadata:
  openclaw:
    requires:
      env: [TRELLO_API_KEY, TRELLO_TOKEN]
---

# Trello REST API

Use the `http` tool. Auth is via query params `key` and `token` appended to every URL.

## Base URL

`https://api.trello.com/1`

Append `?key={TRELLO_API_KEY}&token={TRELLO_TOKEN}` to all URLs.

## Actions

**List my boards:**
```
http(method="GET", url="https://api.trello.com/1/members/me/boards?key={TRELLO_API_KEY}&token={TRELLO_TOKEN}&fields=name,url,shortUrl")
```

**Get board lists:**
```
http(method="GET", url="https://api.trello.com/1/boards/<board_id>/lists?key={TRELLO_API_KEY}&token={TRELLO_TOKEN}&fields=name")
```

**List cards in a list:**
```
http(method="GET", url="https://api.trello.com/1/lists/<list_id>/cards?key={TRELLO_API_KEY}&token={TRELLO_TOKEN}&fields=name,desc,due,labels")
```

**Create card:**
```
http(method="POST", url="https://api.trello.com/1/cards?key={TRELLO_API_KEY}&token={TRELLO_TOKEN}", body={"name": "Card title", "desc": "Description", "idList": "<list_id>", "due": "2026-04-01T12:00:00.000Z"})
```

**Move card to another list:**
```
http(method="PUT", url="https://api.trello.com/1/cards/<card_id>?key={TRELLO_API_KEY}&token={TRELLO_TOKEN}", body={"idList": "<new_list_id>"})
```

**Add comment:**
```
http(method="POST", url="https://api.trello.com/1/cards/<card_id>/actions/comments?key={TRELLO_API_KEY}&token={TRELLO_TOKEN}", body={"text": "Comment"})
```

**Add label to card:**
```
http(method="POST", url="https://api.trello.com/1/cards/<card_id>/idLabels?key={TRELLO_API_KEY}&token={TRELLO_TOKEN}", body={"value": "<label_id>"})
```

**Search:**
```
http(method="GET", url="https://api.trello.com/1/search?key={TRELLO_API_KEY}&token={TRELLO_TOKEN}&query=search+text&modelTypes=cards&cards_limit=10")
```

## Notes

- Trello uses query-parameter auth, not headers. Always include `key` and `token`.
- IDs are 24-character hex strings like `"5a1b2c3d4e5f6a7b8c9d0e1f"`.
- Due dates are ISO 8601 with timezone. `dueComplete: true` marks as done.
- `fields` param controls which properties to return (comma-separated).
- Board shortLinks can be used interchangeably with IDs.
