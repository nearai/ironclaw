---
name: shopify
version: "1.0.0"
description: Shopify Admin API — products, orders, customers, inventory, fulfillment
activation:
  keywords:
    - "shopify"
    - "shopify product"
    - "shopify order"
    - "e-commerce"
  exclude_keywords:
    - "woocommerce"
    - "magento"
  patterns:
    - "(?i)shopify.*(product|order|customer|inventory)"
    - "(?i)(create|list|update).*shopify"
  tags:
    - "e-commerce"
    - "retail"
  max_context_tokens: 1500
metadata:
  openclaw:
    requires:
      env: [SHOPIFY_STORE_DOMAIN, SHOPIFY_ACCESS_TOKEN]
---

# Shopify Admin REST API

Use the `http` tool. Credentials are automatically injected for your Shopify store domain.

## Base URL

`https://{SHOPIFY_STORE_DOMAIN}.myshopify.com/admin/api/2024-01`

## Actions

**List products:**
```
http(method="GET", url="https://{store}.myshopify.com/admin/api/2024-01/products.json?limit=20&status=active")
```

**Get product:**
```
http(method="GET", url="https://{store}.myshopify.com/admin/api/2024-01/products/<product_id>.json")
```

**Create product:**
```
http(method="POST", url="https://{store}.myshopify.com/admin/api/2024-01/products.json", body={"product": {"title": "Cool T-Shirt", "body_html": "<p>Comfortable cotton tee</p>", "vendor": "My Brand", "product_type": "Apparel", "variants": [{"price": "29.99", "sku": "TSHIRT-001", "inventory_quantity": 100}]}})
```

**List orders:**
```
http(method="GET", url="https://{store}.myshopify.com/admin/api/2024-01/orders.json?status=any&limit=20")
```

**Get order:**
```
http(method="GET", url="https://{store}.myshopify.com/admin/api/2024-01/orders/<order_id>.json")
```

**List customers:**
```
http(method="GET", url="https://{store}.myshopify.com/admin/api/2024-01/customers.json?limit=20")
```

**Search customers:**
```
http(method="GET", url="https://{store}.myshopify.com/admin/api/2024-01/customers/search.json?query=email:john@acme.com")
```

**Update inventory level:**
```
http(method="POST", url="https://{store}.myshopify.com/admin/api/2024-01/inventory_levels/set.json", body={"location_id": 123, "inventory_item_id": 456, "available": 50})
```

**Count products:**
```
http(method="GET", url="https://{store}.myshopify.com/admin/api/2024-01/products/count.json")
```

## Notes

- All responses are wrapped in the resource name: `{"products": [...]}` or `{"product": {...}}`.
- Pagination: use `Link` header with `rel="next"` URL. Also `page_info` cursor-based pagination.
- Prices are strings (e.g., `"29.99"`), not numbers.
- Order financial statuses: `paid`, `pending`, `refunded`, `voided`, `partially_refunded`.
- Order fulfillment statuses: `fulfilled`, `partial`, `unfulfilled`, `null`.
- Rate limit: 2 requests/second (leaky bucket). Check `X-Shopify-Shop-Api-Call-Limit` header.
