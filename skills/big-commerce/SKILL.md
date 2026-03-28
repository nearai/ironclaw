---
name: big-commerce
version: "1.0.0"
description: BigCommerce API — BigCommerce is an open SaaS eCommerce platform that enables businesses to build
activation:
  keywords:
    - "big-commerce"
    - "bigcommerce"
    - "ecommerce"
  patterns:
    - "(?i)big.?commerce"
  tags:
    - "tools"
    - "ecommerce"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [BIG_COMMERCE_STORE_HASH, BIG_COMMERCE_ACCESS_TOKEN]
---

# BigCommerce API

Use the `http` tool. API key is automatically injected via `X-Auth-Token` header — **never construct auth headers manually**.

## Base URL

`https://api.bigcommerce.com/stores/{BIGCOMMERCE_STORE_HASH}/v3`

## Actions

**List products:**
```
http(method="GET", url="https://api.bigcommerce.com/stores/{BIGCOMMERCE_STORE_HASH}/v3/catalog/products?limit=10")
```

**Get product:**
```
http(method="GET", url="https://api.bigcommerce.com/stores/{BIGCOMMERCE_STORE_HASH}/v3/catalog/products/{product_id}")
```

**Create product:**
```
http(method="POST", url="https://api.bigcommerce.com/stores/{BIGCOMMERCE_STORE_HASH}/v3/catalog/products", body={"name": "Widget","type": "physical","weight": 1.0,"price": 29.99})
```

**List orders:**
```
http(method="GET", url="https://api.bigcommerce.com/stores/{BIGCOMMERCE_STORE_HASH}/v3/orders?limit=10")
```

**List customers:**
```
http(method="GET", url="https://api.bigcommerce.com/stores/{BIGCOMMERCE_STORE_HASH}/v3/customers?limit=10")
```

## Notes

- Auth via `X-Auth-Token` header (auto-injected).
- Products have `type`: `physical`, `digital`.
- Prices are decimals, weights in the store's configured unit.
