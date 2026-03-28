---
name: shippo
version: "1.0.0"
description: Shippo API — shipping rates, labels, tracking, addresses
activation:
  keywords:
    - "shippo"
    - "shipping label"
    - "shipping rate"
    - "tracking"
  exclude_keywords:
    - "easypost"
  patterns:
    - "(?i)shippo.*(rate|label|track|shipment)"
    - "(?i)(create|get).*shipping.*(label|rate)"
  tags:
    - "shipping"
    - "logistics"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [SHIPPO_API_KEY]
---

# Shippo API

Use the `http` tool. Include `ShippoToken` in Authorization header.

## Base URL

`https://api.goshippo.com`

## Actions

**Create shipment (get rates):**
```
http(method="POST", url="https://api.goshippo.com/shipments", headers=[{"name": "Authorization", "value": "ShippoToken {SHIPPO_API_KEY}"}], body={"address_from": {"name": "Mr Sender", "street1": "123 Main St", "city": "San Francisco", "state": "CA", "zip": "94105", "country": "US"}, "address_to": {"name": "Ms Receiver", "street1": "456 Oak Ave", "city": "New York", "state": "NY", "zip": "10001", "country": "US"}, "parcels": [{"length": "10", "width": "8", "height": "4", "distance_unit": "in", "weight": "2", "mass_unit": "lb"}], "async": false})
```

**Get rates for shipment:**
```
http(method="GET", url="https://api.goshippo.com/shipments/<shipment_id>/rates", headers=[{"name": "Authorization", "value": "ShippoToken {SHIPPO_API_KEY}"}])
```

**Purchase label (from rate):**
```
http(method="POST", url="https://api.goshippo.com/transactions", headers=[{"name": "Authorization", "value": "ShippoToken {SHIPPO_API_KEY}"}], body={"rate": "<rate_object_id>", "label_file_type": "PDF", "async": false})
```

**Track shipment:**
```
http(method="GET", url="https://api.goshippo.com/tracks/<carrier>/<tracking_number>", headers=[{"name": "Authorization", "value": "ShippoToken {SHIPPO_API_KEY}"}])
```

**Validate address:**
```
http(method="POST", url="https://api.goshippo.com/addresses", headers=[{"name": "Authorization", "value": "ShippoToken {SHIPPO_API_KEY}"}], body={"name": "John Doe", "street1": "123 Main St", "city": "San Francisco", "state": "CA", "zip": "94105", "country": "US", "validate": true})
```

**List shipments:**
```
http(method="GET", url="https://api.goshippo.com/shipments?results=20", headers=[{"name": "Authorization", "value": "ShippoToken {SHIPPO_API_KEY}"}])
```

## Notes

- Workflow: Create shipment → select rate → purchase label (transaction).
- Carriers: `usps`, `ups`, `fedex`, `dhl_express`, etc.
- Dimensions are strings, not numbers.
- Label file types: `PDF`, `PNG`, `ZPLII`.
- Test mode: use test API token (rates are test data).
- Transaction status: `SUCCESS`, `QUEUED`, `ERROR`.
