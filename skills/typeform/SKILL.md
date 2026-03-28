---
name: typeform
version: "1.0.0"
description: Typeform API — Typeform is a cloud-based form builder that enables users to create interactive
activation:
  keywords:
    - "typeform"
    - "tools"
  patterns:
    - "(?i)typeform"
  tags:
    - "tools"
    - "utility"
    - "tool"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [TYPEFORM_BASE_URL]
---

# Typeform API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://api.typeform.com`

## Actions

**List forms:**
```
http(method="GET", url="https://api.typeform.com/forms?page_size=10")
```

**Get form:**
```
http(method="GET", url="https://api.typeform.com/forms/{form_id}")
```

**List responses:**
```
http(method="GET", url="https://api.typeform.com/forms/{form_id}/responses?page_size=10")
```

**Create form:**
```
http(method="POST", url="https://api.typeform.com/forms", body={"title": "My Survey","fields": [{"type": "short_text","title": "What is your name?"}]})
```

## Notes

- Field types: `short_text`, `long_text`, `multiple_choice`, `yes_no`, `email`, `number`, `rating`, `date`.
- Responses contain `answers` array matching form `fields`.
- Pagination: `page_size` + `before`/`after` tokens.
- Webhooks configurable per form for real-time responses.
