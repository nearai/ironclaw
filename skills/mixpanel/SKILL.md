---
name: mixpanel
version: "1.0.0"
description: Mixpanel API — events, user profiles, funnels, retention, insights
activation:
  keywords:
    - "mixpanel"
    - "analytics"
    - "mixpanel event"
  exclude_keywords:
    - "amplitude"
    - "google analytics"
  patterns:
    - "(?i)mixpanel.*(event|funnel|retention|insight|profile)"
  tags:
    - "analytics"
    - "product"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [MIXPANEL_PROJECT_TOKEN, MIXPANEL_SERVICE_ACCOUNT]
---

# Mixpanel API

Use the `http` tool. Different endpoints use different auth methods.

## Ingestion API (track events)

**Track event:**
```
http(method="POST", url="https://api.mixpanel.com/track", headers=[{"name": "Content-Type", "value": "application/json"}], body=[{"event": "Purchase", "properties": {"token": "{MIXPANEL_PROJECT_TOKEN}", "distinct_id": "user_123", "amount": 49.99, "item": "Premium Plan"}}])
```

## Query API (analytics)

Base URL: `https://mixpanel.com/api/query`

Requires service account auth (Basic auth with service account credentials).

**Query events (JQL):**
```
http(method="POST", url="https://mixpanel.com/api/query/jql", headers=[{"name": "Authorization", "value": "Basic {MIXPANEL_SERVICE_ACCOUNT}"}], body={"script": "function main() { return Events({from_date: '2026-03-01', to_date: '2026-03-27', event_selectors: [{event: 'Purchase'}]}).groupBy(['properties.item'], mixpanel.reducer.count()); }", "project_id": "<project_id>"})
```

**Get insights:**
```
http(method="GET", url="https://mixpanel.com/api/query/insights?project_id=<id>&bookmark_id=<id>", headers=[{"name": "Authorization", "value": "Basic {MIXPANEL_SERVICE_ACCOUNT}"}])
```

**Export raw events:**
```
http(method="GET", url="https://data.mixpanel.com/api/2.0/export?from_date=2026-03-01&to_date=2026-03-27&event=[\"Purchase\"]", headers=[{"name": "Authorization", "value": "Basic {MIXPANEL_SERVICE_ACCOUNT}"}])
```

## User Profiles

**Set profile properties:**
```
http(method="POST", url="https://api.mixpanel.com/engage#profile-set", body=[{"$token": "{MIXPANEL_PROJECT_TOKEN}", "$distinct_id": "user_123", "$set": {"$name": "Alice", "$email": "alice@example.com", "plan": "premium"}}])
```

## Notes

- Ingestion (track/engage) uses project token in the body, no auth header.
- Query API uses service account Basic auth.
- Export returns newline-delimited JSON (one event per line).
- JQL uses JavaScript-like syntax for custom queries.
- Events are eventually consistent — may take seconds to appear in queries.
