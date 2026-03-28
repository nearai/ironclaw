---
name: ship-engine
version: "1.0.0"
description: ShipEngine API — ShipEngine (rebranded as ShipStation API) is a multi-carrier shipping API that l
activation:
  keywords:
    - "ship-engine"
    - "shipengine"
    - "logistics"
  patterns:
    - "(?i)ship.?engine"
  tags:
    - "tools"
    - "logistics"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [SHIP_ENGINE_API_KEY]
---

# ShipEngine API

Use the `http` tool. API key is automatically injected via `API-Key` header — **never construct auth headers manually**.

## Base URL

`https://api.shipengine.com/v1`

## Actions

**Get rates:**
```
http(method="POST", url="https://api.shipengine.com/v1/rates", body={"shipment": {"ship_from": {"name": "Sender","address_line1": "123 Main St","city_locality": "Austin","state_province": "TX","postal_code": "78701","country_code": "US"},"ship_to": {"name": "Recipient","address_line1": "456 Oak Ave","city_locality": "New York","state_province": "NY","postal_code": "10001","country_code": "US"},"packages": [{"weight": {"value": 2,"unit": "pound"}}]},"rate_options": {"carrier_ids": []}})
```

**Create label:**
```
http(method="POST", url="https://api.shipengine.com/v1/labels", body={"shipment": {"service_code": "usps_priority_mail","ship_from": {},"ship_to": {},"packages": [{"weight": {"value": 2,"unit": "pound"}}]}})
```

**Track package:**
```
http(method="GET", url="https://api.shipengine.com/v1/tracking?carrier_code=usps&tracking_number=1234567890")
```

**Validate address:**
```
http(method="POST", url="https://api.shipengine.com/v1/addresses/validate", body=[{"address_line1": "123 Main St","city_locality": "Austin","state_province": "TX","postal_code": "78701","country_code": "US"}])
```

## Notes

- Auth via `API-Key` header (auto-injected).
- Carrier codes: `usps`, `ups`, `fedex`, `dhl_express`.
- Weight units: `pound`, `ounce`, `gram`, `kilogram`.
- Address validation returns `status`: `verified`, `unverified`, `warning`, `error`.
