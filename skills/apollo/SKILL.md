---
name: apollo
version: "1.0.0"
description: Apollo API — Apollo is a sales intelligence and engagement platform that helps teams find and
activation:
  keywords:
    - "apollo"
    - "crm"
  patterns:
    - "(?i)apollo"
  tags:
    - "crm"
    - "sales"
    - "contacts"
  max_context_tokens: 1200
---

# Apollo API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://api.apollo.io/api/v1`

## Actions

**Search people:**
```
http(method="POST", url="https://api.apollo.io/api/v1/mixed_people/search", body={"person_titles": ["CEO"],"person_locations": ["San Francisco"],"page": 1,"per_page": 10})
```

**Search organizations:**
```
http(method="POST", url="https://api.apollo.io/api/v1/mixed_companies/search", body={"organization_locations": ["United States"],"page": 1,"per_page": 10})
```

**Get person:**
```
http(method="GET", url="https://api.apollo.io/api/v1/people/{person_id}")
```

**Enrich person:**
```
http(method="POST", url="https://api.apollo.io/api/v1/people/match", body={"email": "john@example.com"})
```

## Notes

- API key goes in the request body as `api_key` or as a header.
- Search supports filters: `person_titles`, `person_locations`, `organization_domains`.
- Rate limit: 50 requests per minute on free tier.
