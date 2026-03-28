---
name: intercom
version: "1.0.0"
description: Intercom API — A messaging platform that allows businesses to communicate with prospective and 
activation:
  keywords:
    - "intercom"
    - "ticketing"
  patterns:
    - "(?i)intercom"
  tags:
    - "tools"
    - "Ticketing"
  max_context_tokens: 1200
---

# Intercom API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://api.intercom.io`

## Actions

**List contacts:**
```
http(method="POST", url="https://api.intercom.io/contacts/search", body={"query": {"field": "role","operator": "=","value": "user"},"pagination": {"per_page": 10}})
```

**Get contact:**
```
http(method="GET", url="https://api.intercom.io/contacts/{contact_id}")
```

**Create contact:**
```
http(method="POST", url="https://api.intercom.io/contacts", body={"role": "user","email": "john@example.com","name": "John Doe"})
```

**List conversations:**
```
http(method="GET", url="https://api.intercom.io/conversations?per_page=10")
```

**Send message:**
```
http(method="POST", url="https://api.intercom.io/messages", body={"message_type": "inapp","body": "Hello!","from": {"type": "admin","id": "admin_id"},"to": {"type": "user","id": "user_id"}})
```

## Notes

- Contact roles: `user`, `lead`.
- Search uses nested query objects with operators: `=`, `!=`, `>`, `<`, `contains`, `starts_with`.
- Conversation parts are messages within a conversation.
- Pagination: `starting_after` cursor.
