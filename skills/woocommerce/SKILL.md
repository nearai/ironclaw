---
name: woocommerce
version: "1.0.0"
description: WooCommerce API — WooCommerce is a customizable, open-source eCommerce plugin for WordPress
activation:
  keywords:
    - "woocommerce"
    - "ecommerce"
  patterns:
    - "(?i)woocommerce"
  tags:
    - "tools"
    - "eCommerce"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [WOOCOMMERCE_BASE_URL, WOOCOMMERCE_CONSUMER_KEY, WOOCOMMERCE_CONSUMER_SECRET]
---

# WooCommerce API

Use the `http` tool. Credentials are automatically injected.

## Base URL

`https://{WOOCOMMERCE_STORE}/wp-json/wc/v3`

## Actions

**List products:**
```
http(method="GET", url="https://{WOOCOMMERCE_STORE}/wp-json/wc/v3/products?per_page=10")
```

**Get product:**
```
http(method="GET", url="https://{WOOCOMMERCE_STORE}/wp-json/wc/v3/products/{product_id}")
```

**Create product:**
```
http(method="POST", url="https://{WOOCOMMERCE_STORE}/wp-json/wc/v3/products", body={"name": "Widget","type": "simple","regular_price": "29.99","description": "A great widget"})
```

**List orders:**
```
http(method="GET", url="https://{WOOCOMMERCE_STORE}/wp-json/wc/v3/orders?per_page=10")
```

**List customers:**
```
http(method="GET", url="https://{WOOCOMMERCE_STORE}/wp-json/wc/v3/customers?per_page=10")
```

## Notes

- Uses Basic auth with consumer key and secret.
- Product types: `simple`, `grouped`, `variable`, `external`.
- Order statuses: `pending`, `processing`, `on-hold`, `completed`, `cancelled`, `refunded`, `failed`.
- Pagination: `page` and `per_page` params; check `X-WP-Total` header.
