---
name: stripe
version: "1.0.0"
description: Stripe API — payments, customers, subscriptions, invoices, products
activation:
  keywords:
    - "stripe"
    - "payment"
    - "subscription"
    - "invoice"
    - "charge"
  exclude_keywords:
    - "paypal"
    - "square"
  patterns:
    - "(?i)stripe.*(customer|payment|subscription|invoice|charge)"
    - "(?i)(create|list|cancel).*subscription"
    - "(?i)(charge|refund|payment)"
  tags:
    - "payments"
    - "billing"
    - "finance"
  max_context_tokens: 1800
metadata:
  openclaw:
    requires:
      env: [STRIPE_SECRET_KEY]
---

# Stripe API

Use the `http` tool. Credentials are automatically injected for `api.stripe.com`. Stripe uses **form-encoded bodies** for POST/PUT — pass the body as a flat object and include `Content-Type: application/x-www-form-urlencoded`.

## Base URL

`https://api.stripe.com/v1`

**Important**: Stripe expects form-encoded POST data. Use URL-encoded key-value pairs or nested bracket notation.

## Actions

**List customers:**
```
http(method="GET", url="https://api.stripe.com/v1/customers?limit=10")
```

**Create customer:**
```
http(method="POST", url="https://api.stripe.com/v1/customers", headers=[{"name": "Content-Type", "value": "application/x-www-form-urlencoded"}], body="name=John+Doe&email=john@acme.com&metadata[company]=Acme")
```

**Create payment intent:**
```
http(method="POST", url="https://api.stripe.com/v1/payment_intents", headers=[{"name": "Content-Type", "value": "application/x-www-form-urlencoded"}], body="amount=5000&currency=usd&customer=cus_xxx&description=Order+123")
```

**List subscriptions:**
```
http(method="GET", url="https://api.stripe.com/v1/subscriptions?customer=cus_xxx&status=active&limit=10")
```

**Create subscription:**
```
http(method="POST", url="https://api.stripe.com/v1/subscriptions", headers=[{"name": "Content-Type", "value": "application/x-www-form-urlencoded"}], body="customer=cus_xxx&items[0][price]=price_xxx")
```

**Cancel subscription:**
```
http(method="DELETE", url="https://api.stripe.com/v1/subscriptions/sub_xxx")
```

**List invoices:**
```
http(method="GET", url="https://api.stripe.com/v1/invoices?customer=cus_xxx&limit=10")
```

**Create refund:**
```
http(method="POST", url="https://api.stripe.com/v1/refunds", headers=[{"name": "Content-Type", "value": "application/x-www-form-urlencoded"}], body="payment_intent=pi_xxx&amount=2000")
```

**List products:**
```
http(method="GET", url="https://api.stripe.com/v1/products?active=true&limit=20")
```

**Get balance:**
```
http(method="GET", url="https://api.stripe.com/v1/balance")
```

## Notes

- All amounts are in the **smallest currency unit** (cents for USD): $50.00 = `5000`.
- IDs are prefixed: `cus_` (customer), `sub_` (subscription), `pi_` (payment intent), `price_` (price), `prod_` (product), `in_` (invoice).
- Nested params use bracket notation: `items[0][price]=price_xxx`.
- Pagination: `starting_after=<last_id>` + `limit`. Check `has_more` in response.
- List responses: `{"data": [...], "has_more": true/false}`.
- Use test keys (prefix `sk_test_`) for development.
