---
name: google-places
version: "1.0.0"
description: Google Places API — Google Places is a location-based service that provides detailed information abo
activation:
  keywords:
    - "google-places"
    - "google places"
    - "tools"
  patterns:
    - "(?i)google.?places"
  tags:
    - "tools"
    - "utility"
  max_context_tokens: 1200
---

# Google Places API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://places.googleapis.com/v1`

**Required headers**: `X-Goog-FieldMask: places.displayName,places.formattedAddress,places.rating`

## Actions

**Search places:**
```
http(method="POST", url="https://places.googleapis.com/v1/places:searchText", headers=[{"name": "X-Goog-FieldMask","value": "places.displayName,places.formattedAddress,places.rating"}], body={"textQuery": "restaurants in San Francisco","maxResultCount": 10})
```

**Get place details:**
```
http(method="GET", url="https://places.googleapis.com/v1/places/{place_id}")
```

**Nearby search:**
```
http(method="POST", url="https://places.googleapis.com/v1/places:searchNearby", headers=[{"name": "X-Goog-FieldMask","value": "places.displayName,places.formattedAddress,places.rating"}], body={"locationRestriction": {"circle": {"center": {"latitude": 37.7749,"longitude": -122.4194},"radius": 1000.0}},"includedTypes": ["restaurant"],"maxResultCount": 10})
```

## Notes

- Requires `X-Goog-FieldMask` header to select response fields.
- Place types: `restaurant`, `cafe`, `hotel`, `hospital`, `park`, etc.
- API key in `X-Goog-Api-Key` header or query param `key`.
