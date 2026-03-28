---
name: ship-station
version: "1.0.0"
description: ShipStation API — ShipStation is a web-based shipping and fulfillment platform with a powerful dev
activation:
  keywords:
    - "ship-station"
    - "shipstation"
    - "logistics"
  patterns:
    - "(?i)ship.?station"
  tags:
    - "tools"
    - "logistics"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [SHIP_STATION_API_KEY]
---

# ShipStation API

Use the `http` tool. API key is automatically injected via `API-Key` header — **never construct auth headers manually**.

## Base URL

`https://ssapi.shipstation.com`

## Actions

**List orders:**
```
http(method="GET", url="https://ssapi.shipstation.com/orders?pageSize=10&page=1")
```

**Get order:**
```
http(method="GET", url="https://ssapi.shipstation.com/orders/{order_id}")
```

**Create order:**
```
http(method="POST", url="https://ssapi.shipstation.com/orders/createorder", body={"orderNumber": "ORD-001","orderDate": "2026-03-27","orderStatus": "awaiting_shipment","billTo": {"name": "John Doe"},"shipTo": {"name": "John Doe","street1": "123 Main St","city": "Austin","state": "TX","postalCode": "78701","country": "US"}})
```

**List shipments:**
```
http(method="GET", url="https://ssapi.shipstation.com/shipments?pageSize=10&page=1")
```

**List carriers:**
```
http(method="GET", url="https://ssapi.shipstation.com/carriers")
```

## Notes

- Uses Basic auth with API key and secret.
- Order statuses: `awaiting_payment`, `awaiting_shipment`, `shipped`, `on_hold`, `cancelled`.
- Pagination: `page` and `pageSize` (max 500).
- Dates in `YYYY-MM-DD` format.
