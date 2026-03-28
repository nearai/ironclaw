---
name: datadog
version: "1.0.0"
description: Datadog API — monitors, events, metrics, dashboards, incidents
activation:
  keywords:
    - "datadog"
    - "monitoring"
    - "datadog alert"
    - "datadog monitor"
  exclude_keywords:
    - "sentry"
    - "grafana"
    - "new relic"
  patterns:
    - "(?i)datadog.*(monitor|alert|metric|dashboard|incident)"
    - "(?i)(create|list|mute).*monitor"
  tags:
    - "monitoring"
    - "observability"
    - "devops"
  max_context_tokens: 1500
metadata:
  openclaw:
    requires:
      env: [DATADOG_API_KEY, DATADOG_APP_KEY]
---

# Datadog API

Use the `http` tool. Include both API and application keys as headers.

## Base URL

`https://api.datadoghq.com/api` (US1). Other sites: `api.us3.datadoghq.com`, `api.datadoghq.eu`, etc.

Required headers on every request:
```
headers=[{"name": "DD-API-KEY", "value": "{DATADOG_API_KEY}"}, {"name": "DD-APPLICATION-KEY", "value": "{DATADOG_APP_KEY}"}]
```

## Actions

**List monitors:**
```
http(method="GET", url="https://api.datadoghq.com/api/v1/monitor?page=0&page_size=20", headers=[{"name": "DD-API-KEY", "value": "{DATADOG_API_KEY}"}, {"name": "DD-APPLICATION-KEY", "value": "{DATADOG_APP_KEY}"}])
```

**Get monitor:**
```
http(method="GET", url="https://api.datadoghq.com/api/v1/monitor/<monitor_id>", headers=[{"name": "DD-API-KEY", "value": "{DATADOG_API_KEY}"}, {"name": "DD-APPLICATION-KEY", "value": "{DATADOG_APP_KEY}"}])
```

**Mute monitor:**
```
http(method="POST", url="https://api.datadoghq.com/api/v1/monitor/<monitor_id>/mute", headers=[{"name": "DD-API-KEY", "value": "{DATADOG_API_KEY}"}, {"name": "DD-APPLICATION-KEY", "value": "{DATADOG_APP_KEY}"}], body={"end": 1775000000})
```

**Post event:**
```
http(method="POST", url="https://api.datadoghq.com/api/v1/events", headers=[{"name": "DD-API-KEY", "value": "{DATADOG_API_KEY}"}], body={"title": "Deploy completed", "text": "Version 2.1.0 deployed to production", "alert_type": "info", "tags": ["env:production", "service:api"]})
```

**Query metrics:**
```
http(method="GET", url="https://api.datadoghq.com/api/v1/query?from=1775000000&to=1775100000&query=avg:system.cpu.user{host:myhost}", headers=[{"name": "DD-API-KEY", "value": "{DATADOG_API_KEY}"}, {"name": "DD-APPLICATION-KEY", "value": "{DATADOG_APP_KEY}"}])
```

**List dashboards:**
```
http(method="GET", url="https://api.datadoghq.com/api/v1/dashboard/lists/manual", headers=[{"name": "DD-API-KEY", "value": "{DATADOG_API_KEY}"}, {"name": "DD-APPLICATION-KEY", "value": "{DATADOG_APP_KEY}"}])
```

**List incidents:**
```
http(method="GET", url="https://api.datadoghq.com/api/v2/incidents?page[size]=10", headers=[{"name": "DD-API-KEY", "value": "{DATADOG_API_KEY}"}, {"name": "DD-APPLICATION-KEY", "value": "{DATADOG_APP_KEY}"}])
```

## Notes

- Monitor states: `OK`, `Alert`, `Warn`, `No Data`, `Unknown`.
- Event alert types: `info`, `warning`, `error`, `success`.
- Metric queries use Datadog query syntax: `avg:metric.name{tag:value}`.
- Times are Unix timestamps (seconds).
- `DD-API-KEY` alone suffices for posting events/metrics. Most reads need `DD-APPLICATION-KEY` too.
