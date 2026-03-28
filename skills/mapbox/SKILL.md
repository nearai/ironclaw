---
name: mapbox
version: "1.0.0"
description: Mapbox API — Mapbox is a developer-focused mapping platform that provides customizable
activation:
  keywords:
    - "mapbox"
    - "tools"
  patterns:
    - "(?i)mapbox"
  tags:
    - "tools"
    - "utility"
    - "tool"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [MAPBOX_ACCESS_TOKEN]
---

# Mapbox API

Use the `http` tool. API key is automatically injected as `access_token` query parameter.

## Base URL

`https://api.mapbox.com`

## Actions

**Geocode address:**
```
http(method="GET", url="https://api.mapbox.com/geocoding/v5/mapbox.places/San+Francisco.json?access_token={MAPBOX_ACCESS_TOKEN}&limit=5")
```

**Reverse geocode:**
```
http(method="GET", url="https://api.mapbox.com/geocoding/v5/mapbox.places/-122.4194,37.7749.json?access_token={MAPBOX_ACCESS_TOKEN}")
```

**Get directions:**
```
http(method="GET", url="https://api.mapbox.com/directions/v5/mapbox/driving/-122.4194,37.7749;-118.2437,34.0522?access_token={MAPBOX_ACCESS_TOKEN}&geometries=geojson")
```

**Get static map:**
```
http(method="GET", url="https://api.mapbox.com/styles/v1/mapbox/streets-v12/static/-122.4194,37.7749,12,0/600x400?access_token={MAPBOX_ACCESS_TOKEN}")
```

## Notes

- Access token as query parameter `access_token`.
- Coordinates are `longitude,latitude` (note the order).
- Profiles: `mapbox/driving`, `mapbox/walking`, `mapbox/cycling`, `mapbox/driving-traffic`.
- Geocoding returns GeoJSON FeatureCollection.
