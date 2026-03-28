---
name: google-routes
version: "1.0.0"
description: Google Routes API — Google Routes helps you calculate efficient
activation:
  keywords:
    - "google-routes"
    - "google routes"
    - "maps"
  patterns:
    - "(?i)google.?routes"
  tags:
    - "maps"
    - "location"
    - "geolocation"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [GOOGLE_ROUTES_API_KEY]
---

# Google Routes API

Use the `http` tool. API key is automatically injected via `X-Goog-Api-Key` header — **never construct auth headers manually**.

## Base URL

`https://routes.googleapis.com`

**Required headers**: `X-Goog-FieldMask: routes.duration,routes.distanceMeters,routes.polyline`

## Actions

**Compute route:**
```
http(method="POST", url="https://routes.googleapis.com/directions/v2:computeRoutes", headers=[{"name": "X-Goog-FieldMask","value": "routes.duration,routes.distanceMeters,routes.polyline"}], body={"origin": {"location": {"latLng": {"latitude": 37.7749,"longitude": -122.4194}}},"destination": {"location": {"latLng": {"latitude": 34.0522,"longitude": -118.2437}}},"travelMode": "DRIVE"})
```

**Compute route matrix:**
```
http(method="POST", url="https://routes.googleapis.com/distanceMatrix/v2:computeRouteMatrix", headers=[{"name": "X-Goog-FieldMask","value": "routes.duration,routes.distanceMeters,routes.polyline"}], body={"origins": [{"waypoint": {"location": {"latLng": {"latitude": 37.7749,"longitude": -122.4194}}}}],"destinations": [{"waypoint": {"location": {"latLng": {"latitude": 34.0522,"longitude": -118.2437}}}}],"travelMode": "DRIVE"})
```

## Notes

- Requires `X-Goog-FieldMask` header.
- Travel modes: `DRIVE`, `BICYCLE`, `WALK`, `TWO_WHEELER`, `TRANSIT`.
- Supports waypoints, route modifiers (avoid tolls/highways), and departure time.
