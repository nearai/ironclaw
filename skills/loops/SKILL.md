---
name: loops
version: "1.0.0"
description: Loops API — Loops is a modern email marketing and automation platform designed for developer
activation:
  keywords:
    - "loops"
    - "tools"
  patterns:
    - "(?i)loops"
  tags:
    - "tools"
    - "utility"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [LOOPS_API_KEY]
---

# Loops API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://app.loops.so/api/v1`

## Actions

**Create contact:**
```
http(method="POST", url="https://app.loops.so/api/v1/contacts/create", body={"email": "john@example.com","firstName": "John","lastName": "Doe","source": "api"})
```

**Update contact:**
```
http(method="PUT", url="https://app.loops.so/api/v1/contacts/update", body={"email": "john@example.com","firstName": "Jane"})
```

**Send event:**
```
http(method="POST", url="https://app.loops.so/api/v1/events/send", body={"email": "john@example.com","eventName": "signup_completed"})
```

**Send transactional email:**
```
http(method="POST", url="https://app.loops.so/api/v1/transactional", body={"transactionalId": "template_id","email": "john@example.com","dataVariables": {"name": "John"}})
```

## Notes

- Email is the primary identifier for contacts.
- Events trigger automations configured in the Loops dashboard.
- Transactional emails use pre-built templates by ID.
- Custom properties can be added as top-level fields on contacts.
