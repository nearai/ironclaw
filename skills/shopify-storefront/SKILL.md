---
name: shopify-storefront
version: "1.0.0"
description: Shopify Storefront API — Shopify Storefront allows you to create custom shopping experiences using Shopif
activation:
  keywords:
    - "shopify-storefront"
    - "shopify storefront"
    - "ecommerce"
  patterns:
    - "(?i)shopify.?storefront"
  tags:
    - "tools"
    - "ecommerce"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [SHOPIFY_STORE_NAME, SHOPIFY_STOREFRONT_ACCESS_TOKEN]
---

# Shopify Storefront API

Use the `http` tool. API key is automatically injected via `X-Shopify-Storefront-Access-Token` header — **never construct auth headers manually**.

## Base URL

`https://{SHOPIFY_STORE}.myshopify.com/api/2024-10/graphql.json`

## Actions

**List products:**
```
http(method="POST", url="https://{SHOPIFY_STORE}.myshopify.com/api/2024-10/graphql.json", body={"query": "{ products(first: 10) { edges { node { id title handle priceRange { minVariantPrice { amount currencyCode } } } } } }"})
```

**Get product by handle:**
```
http(method="POST", url="https://{SHOPIFY_STORE}.myshopify.com/api/2024-10/graphql.json", body={"query": "{ productByHandle(handle: \"widget\") { id title description variants(first: 5) { edges { node { id title price { amount currencyCode } } } } } }"})
```

## Notes

- GraphQL-only API for customer-facing operations.
- Auth via `X-Shopify-Storefront-Access-Token` header (auto-injected).
- IDs are globally unique: `gid://shopify/Product/123`.
- Supports cart, checkout, and customer operations.
